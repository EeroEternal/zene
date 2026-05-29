use anyhow::{Context, Result};
use tracing::info;
use zene_config::CompactionConfig;
use zene_llm::{ChatClient, ChatRequest, ChatResponse, Message, Role, ToolDefinition};
use zene_session::SessionRecord;

use crate::tokens;

const SUMMARY_SYSTEM_PROMPT: &str = "You summarize coding agent conversations. Preserve key user requests, files discussed or modified, tool outcomes, and current task state. Be concise.";

pub fn should_compact(estimated_tokens: u32, config: &CompactionConfig) -> bool {
    let threshold =
        (config.context_window_tokens as f32 * config.trigger_ratio).floor() as u32;
    estimated_tokens >= threshold
}

pub fn keep_recent_token_budget(config: &CompactionConfig) -> u32 {
    (config.context_window_tokens as f32 * config.keep_recent_ratio).floor() as u32
}

/// Index where the recent tail begins. Returns `None` if there is nothing worth compacting.
pub fn tail_start_index(messages: &[Message], keep_recent_tokens: u32) -> Option<usize> {
    if messages.is_empty() {
        return None;
    }

    let prefix_start = messages
        .first()
        .filter(|m| m.role == Role::System)
        .map(|_| 1usize)
        .unwrap_or(0);

    if prefix_start >= messages.len() {
        return None;
    }

    let mut tokens = 0u32;
    let mut tail_start = messages.len();

    for i in (prefix_start..messages.len()).rev() {
        tokens += tokens::estimate_message_tokens(&messages[i]);
        tail_start = i;
        if tokens >= keep_recent_tokens {
            break;
        }
    }

    if tail_start <= prefix_start {
        None
    } else {
        Some(tail_start)
    }
}

pub fn is_context_overflow_error(err: &anyhow::Error) -> bool {
    let msg = err.to_string().to_lowercase();
    msg.contains("context length")
        || msg.contains("context window")
        || msg.contains("maximum context")
        || msg.contains("max context")
        || msg.contains("token limit")
        || msg.contains("too many tokens")
        || msg.contains("context overflow")
        || msg.contains("prompt is too long")
        || msg.contains("request too large")
}

fn format_messages_for_summary(messages: &[Message]) -> String {
    let mut out = String::new();
    for message in messages {
        let role = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };
        out.push_str(&format!("[{role}] "));
        if let Some(content) = &message.content {
            out.push_str(content);
            out.push('\n');
        }
        if let Some(tool_calls) = &message.tool_calls {
            for call in tool_calls {
                out.push_str(&format!("tool_call: {}({})\n", call.name, call.arguments));
            }
        }
        if message.role == Role::Tool {
            if let Some(name) = &message.name {
                out.push_str(&format!("tool: {name}\n"));
            }
        }
        out.push('\n');
    }
    out
}

pub async fn summarize_messages(
    client: &ChatClient,
    model: &str,
    messages: &[Message],
) -> Result<String> {
    let conversation = format_messages_for_summary(messages);
    let request = ChatRequest {
        model: model.to_string(),
        messages: vec![
            Message::system(SUMMARY_SYSTEM_PROMPT),
            Message::user(format!(
                "Summarize this conversation for a coding agent to continue work:\n\n{conversation}"
            )),
        ],
        tools: Vec::<ToolDefinition>::new(),
        stream: false,
    };

    let response = client.chat(request).await.context("compaction summary")?;
    Ok(response
        .message
        .content
        .unwrap_or_else(|| "(empty summary)".to_string()))
}

pub struct CompactionPlan {
    pub tail_start: usize,
    pub compacted_count: usize,
}

pub struct CompactionResult {
    pub reason: String,
    pub compacted_count: usize,
}

pub fn plan_compaction(messages: &[Message], config: &CompactionConfig) -> Option<CompactionPlan> {
    let tail_start = tail_start_index(messages, keep_recent_token_budget(config))?;
    let prefix_start = messages
        .first()
        .filter(|m| m.role == Role::System)
        .map(|_| 1usize)
        .unwrap_or(0);
    let compacted_count = tail_start.saturating_sub(prefix_start);
    if compacted_count == 0 {
        return None;
    }
    Some(CompactionPlan {
        tail_start,
        compacted_count,
    })
}

pub fn subagent_compaction_config(parent: &CompactionConfig) -> CompactionConfig {
    CompactionConfig {
        context_window_tokens: parent.context_window_tokens.min(16_000),
        trigger_ratio: 0.5,
        keep_recent_ratio: parent.keep_recent_ratio.max(0.3),
    }
}

pub fn apply_compaction_to_messages(
    messages: &mut Vec<Message>,
    summary: String,
    tail_start: usize,
    compacted_count: usize,
) {
    let system = messages
        .first()
        .filter(|m| m.role == Role::System)
        .cloned();
    let tail = messages[tail_start..].to_vec();
    let summary_message = Message::compaction_summary(format!(
        "[Previous conversation summary]\n{summary}"
    ));

    messages.clear();
    if let Some(system) = system {
        messages.push(system);
    }
    messages.push(summary_message);
    messages.extend(tail);
    let _ = compacted_count;
}

pub async fn compact_message_list_with_chat<F, Fut>(
    messages: &mut Vec<Message>,
    model: &str,
    config: &CompactionConfig,
    reason: &str,
    chat: F,
) -> Result<Option<CompactionResult>>
where
    F: FnOnce(ChatRequest) -> Fut,
    Fut: std::future::Future<Output = Result<ChatResponse>>,
{
    let plan = match plan_compaction(messages, config) {
        Some(plan) => plan,
        None => return Ok(None),
    };

    let prefix_start = messages
        .first()
        .filter(|m| m.role == Role::System)
        .map(|_| 1usize)
        .unwrap_or(0);
    let prefix = &messages[prefix_start..plan.tail_start];
    let summary = summarize_messages_with_chat(model, prefix, chat).await?;

    info!(
        reason = reason,
        compacted_messages = plan.compacted_count,
        tail_messages = messages.len() - plan.tail_start,
        summary_chars = summary.len(),
        "subagent context compaction applied"
    );

    apply_compaction_to_messages(messages, summary, plan.tail_start, plan.compacted_count);
    Ok(Some(CompactionResult {
        reason: reason.to_string(),
        compacted_count: plan.compacted_count,
    }))
}

pub async fn summarize_messages_with_chat<F, Fut>(
    model: &str,
    messages: &[Message],
    chat: F,
) -> Result<String>
where
    F: FnOnce(ChatRequest) -> Fut,
    Fut: std::future::Future<Output = Result<ChatResponse>>,
{
    let conversation = format_messages_for_summary(messages);
    let request = ChatRequest {
        model: model.to_string(),
        messages: vec![
            Message::system(SUMMARY_SYSTEM_PROMPT),
            Message::user(format!(
                "Summarize this conversation for a coding agent to continue work:\n\n{conversation}"
            )),
        ],
        tools: Vec::<ToolDefinition>::new(),
        stream: false,
    };

    let response = chat(request).await.context("compaction summary")?;
    Ok(response
        .message
        .content
        .unwrap_or_else(|| "(empty summary)".to_string()))
}

pub async fn compact_session(
    session: &mut SessionRecord,
    client: &ChatClient,
    model: &str,
    config: &CompactionConfig,
    reason: &str,
) -> Result<Option<CompactionResult>> {
    let plan = match plan_compaction(&session.messages, config) {
        Some(plan) => plan,
        None => return Ok(None),
    };

    let prefix_start = session
        .messages
        .first()
        .filter(|m| m.role == Role::System)
        .map(|_| 1usize)
        .unwrap_or(0);
    let prefix = &session.messages[prefix_start..plan.tail_start];
    let summary = summarize_messages(client, model, prefix).await?;

    info!(
        reason = reason,
        compacted_messages = plan.compacted_count,
        tail_messages = session.messages.len() - plan.tail_start,
        summary_chars = summary.len(),
        "context compaction applied"
    );

    session.replace_messages_after_compaction(summary, plan.tail_start, plan.compacted_count);
    Ok(Some(CompactionResult {
        reason: reason.to_string(),
        compacted_count: plan.compacted_count,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_config::CompactionConfig;
    use zene_llm::ToolCall;

    fn user_msg(text: &str) -> Message {
        Message::user(text)
    }

    fn assistant_msg(text: &str) -> Message {
        Message::assistant(text)
    }

    #[test]
    fn should_compact_when_over_threshold() {
        let config = CompactionConfig {
            trigger_ratio: 0.5,
            keep_recent_ratio: 0.25,
            context_window_tokens: 1000,
        };
        assert!(!should_compact(499, &config));
        assert!(should_compact(500, &config));
        assert!(should_compact(600, &config));
    }

    #[test]
    fn tail_start_keeps_recent_budget() {
        let messages = vec![
            Message::system("sys"),
            user_msg(&"x".repeat(400)),
            assistant_msg(&"y".repeat(400)),
            user_msg(&"z".repeat(400)),
            assistant_msg("recent"),
        ];
        let tail_start = tail_start_index(&messages, 100).expect("tail start");
        assert!(tail_start >= 3);
        assert!(tail_start < messages.len());
    }

    #[test]
    fn tail_start_none_when_too_few_messages() {
        let messages = vec![Message::system("sys"), user_msg("hi")];
        assert!(tail_start_index(&messages, 100).is_none());
    }

    #[test]
    fn plan_compaction_counts_prefix_messages() {
        let messages = vec![
            Message::system("sys"),
            user_msg(&"a".repeat(200)),
            assistant_msg(&"b".repeat(200)),
            user_msg(&"c".repeat(200)),
            assistant_msg("recent"),
        ];
        let config = CompactionConfig {
            trigger_ratio: 0.85,
            keep_recent_ratio: 0.5,
            context_window_tokens: 100,
        };
        let plan = plan_compaction(&messages, &config).expect("plan");
        assert!(plan.compacted_count >= 1);
        assert!(plan.tail_start > 1);
    }

    #[test]
    fn subagent_compaction_config_uses_smaller_threshold() {
        let parent = CompactionConfig {
            trigger_ratio: 0.85,
            keep_recent_ratio: 0.25,
            context_window_tokens: 128_000,
        };
        let sub = subagent_compaction_config(&parent);
        assert_eq!(sub.context_window_tokens, 16_000);
        assert_eq!(sub.trigger_ratio, 0.5);
    }

    #[test]
    fn overflow_error_detection() {
        assert!(is_context_overflow_error(&anyhow::anyhow!(
            "maximum context length exceeded"
        )));
        assert!(is_context_overflow_error(&anyhow::anyhow!("prompt is too long")));
        assert!(!is_context_overflow_error(&anyhow::anyhow!("connection reset")));
    }

    #[test]
    fn assistant_with_tools_in_summary_format() {
        let messages = vec![Message::assistant_with_tools(
            None,
            vec![ToolCall {
                id: "1".into(),
                name: "Read".into(),
                arguments: "{}".into(),
            }],
        )];
        let formatted = format_messages_for_summary(&messages);
        assert!(formatted.contains("tool_call: Read"));
    }
}
