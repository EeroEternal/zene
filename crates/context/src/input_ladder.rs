//! Compaction input ladder: verbatim → fitted → lossy (grok-aligned).
//!
//! - **Verbatim**: keep tool I/O and bodies (best for quality / cache-shaped prefix).
//! - **Fitted**: drop oldest whole turns (never split tool pairs), mild body shrink.
//! - **Lossy**: flatten tool results to names, aggressive shrink, then fit to budget.

use zene_llm::{Message, MessageKind, Role};

use crate::tokens::TokenEstimator;

/// Stage of the summarization input ladder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputLadderStage {
    Verbatim,
    Fitted,
    Lossy,
}

impl InputLadderStage {
    pub fn next(self) -> Option<Self> {
        match self {
            Self::Verbatim => Some(Self::Fitted),
            Self::Fitted => Some(Self::Lossy),
            Self::Lossy => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verbatim => "verbatim",
            Self::Fitted => "fitted",
            Self::Lossy => "lossy",
        }
    }
}

const FITTED_TOOL_CHARS: usize = 400;
const FITTED_ASSISTANT_CHARS: usize = 600;
const LOSSY_ASSISTANT_CHARS: usize = 200;

fn truncate_body(content: &str, max_chars: usize) -> String {
    let count = content.chars().count();
    if count <= max_chars {
        return content.to_string();
    }
    let kept: String = content.chars().take(max_chars).collect();
    format!("{kept}…[truncated {count} chars]")
}

fn shrink_message(message: &Message, tool_max: usize, assistant_max: usize) -> Message {
    let mut out = message.clone();
    if let Some(content) = out.content.as_ref() {
        let max = match out.role {
            Role::Tool => tool_max,
            Role::Assistant if out.kind != Some(MessageKind::CompactionSummary) => assistant_max,
            Role::User => assistant_max.saturating_mul(2),
            _ => content.chars().count(),
        };
        if content.chars().count() > max {
            out.content = Some(truncate_body(content, max));
        }
    }
    out
}

fn flatten_tool_result(message: &Message) -> Message {
    let mut out = message.clone();
    if out.role == Role::Tool {
        let name = out.name.as_deref().unwrap_or("tool");
        let bytes = out.content.as_ref().map(|c| c.len()).unwrap_or(0);
        out.content = Some(format!(
            "[{name} result omitted for compaction; {bytes} bytes]"
        ));
    }
    out
}

/// Drop oldest non-system messages until under budget, never splitting tool pairs.
pub fn fit_messages_to_budget(
    messages: &[Message],
    budget: u32,
    estimator: &TokenEstimator,
) -> Vec<Message> {
    let mut out = messages.to_vec();
    while estimator.estimate_messages_tokens(&out) > budget && out.len() > 2 {
        let drop_at = if out.first().is_some_and(|m| m.role == Role::System) {
            1
        } else {
            0
        };
        if drop_at >= out.len().saturating_sub(1) {
            break;
        }
        // If dropping would leave orphan tool results, drop the whole pair.
        let mut end = drop_at + 1;
        if out[drop_at].role == Role::Assistant
            && out[drop_at]
                .tool_calls
                .as_ref()
                .is_some_and(|c| !c.is_empty())
        {
            while end < out.len() && out[end].role == Role::Tool {
                end += 1;
            }
        }
        if end >= out.len() {
            break;
        }
        out.drain(drop_at..end);
    }
    out
}

/// Prepare messages for the summarizer at the given ladder stage.
pub fn prepare_summary_input(
    messages: &[Message],
    stage: InputLadderStage,
    token_budget: Option<u32>,
    estimator: &TokenEstimator,
) -> Vec<Message> {
    let prepared: Vec<Message> = match stage {
        InputLadderStage::Verbatim => messages.to_vec(),
        InputLadderStage::Fitted => messages
            .iter()
            .map(|m| shrink_message(m, FITTED_TOOL_CHARS, FITTED_ASSISTANT_CHARS))
            .collect(),
        InputLadderStage::Lossy => messages
            .iter()
            .map(|m| {
                let flat = flatten_tool_result(m);
                shrink_message(&flat, 80, LOSSY_ASSISTANT_CHARS)
            })
            .collect(),
    };

    let Some(budget) = token_budget else {
        return prepared;
    };
    fit_messages_to_budget(&prepared, budget, estimator)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::{Message, ToolCall};

    #[test]
    fn fitted_shortens_tool_bodies() {
        let messages = vec![
            Message::system("sys"),
            Message::tool_result("1", "Read", "x".repeat(2000)),
            Message::user("continue"),
        ];
        let fitted = prepare_summary_input(
            &messages,
            InputLadderStage::Fitted,
            None,
            &TokenEstimator::default(),
        );
        let tool = fitted[1].content.as_deref().unwrap();
        assert!(tool.contains("truncated"));
        assert!(tool.chars().count() < 500);
    }

    #[test]
    fn lossy_flattens_tool_results() {
        let messages = vec![
            Message::system("sys"),
            Message::assistant_with_tools(
                None,
                vec![ToolCall {
                    id: "1".into(),
                    name: "Read".into(),
                    arguments: "{}".into(),
                }],
            ),
            Message::tool_result("1", "Read", "x".repeat(5000)),
            Message::user("continue"),
        ];
        let lossy = prepare_summary_input(
            &messages,
            InputLadderStage::Lossy,
            Some(400),
            &TokenEstimator::default(),
        );
        let tool = lossy
            .iter()
            .find(|m| m.role == Role::Tool)
            .and_then(|m| m.content.as_deref())
            .unwrap_or("");
        assert!(tool.contains("omitted for compaction"));
    }

    #[test]
    fn fit_budget_keeps_system() {
        let mut messages = vec![Message::system("sys")];
        for i in 0..20 {
            messages.push(Message::user(format!("msg {i} {}", "x".repeat(80))));
        }
        let fitted = prepare_summary_input(
            &messages,
            InputLadderStage::Fitted,
            Some(200),
            &TokenEstimator::default(),
        );
        assert!(fitted.len() < messages.len());
        assert_eq!(fitted[0].role, Role::System);
    }

    #[test]
    fn stage_advances() {
        assert_eq!(
            InputLadderStage::Verbatim.next(),
            Some(InputLadderStage::Fitted)
        );
        assert_eq!(InputLadderStage::Lossy.next(), None);
    }
}
