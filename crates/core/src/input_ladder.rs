//! Compaction input ladder: verbatim → fitted → lossy.
//!
//! When summarizing history, start with the full prefix. On context-length
//! overflow (or when the fitted budget is exceeded), step down to a truncated
//! then aggressively reduced view so compaction itself can fit.

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
const LOSSY_TOOL_CHARS: usize = 120;
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
        InputLadderStage::Lossy => {
            let shrunk: Vec<Message> = messages
                .iter()
                .map(|m| shrink_message(m, LOSSY_TOOL_CHARS, LOSSY_ASSISTANT_CHARS))
                .collect();
            // Keep only the most recent half of non-system messages when still large.
            if shrunk.len() > 6 {
                let keep_from = shrunk.len() / 2;
                let mut out = Vec::new();
                if shrunk
                    .first()
                    .is_some_and(|m| m.role == Role::System)
                {
                    out.push(shrunk[0].clone());
                    out.extend(shrunk[keep_from.max(1)..].iter().cloned());
                } else {
                    out.extend(shrunk[keep_from..].iter().cloned());
                }
                out
            } else {
                shrunk
            }
        }
    };

    let Some(budget) = token_budget else {
        return prepared;
    };

    // Trim from the front (after system) until under budget.
    let mut out = prepared;
    while estimator.estimate_messages_tokens(&out) > budget && out.len() > 2 {
        let drop_at = if out.first().is_some_and(|m| m.role == Role::System) {
            1
        } else {
            0
        };
        if drop_at >= out.len().saturating_sub(1) {
            break;
        }
        out.remove(drop_at);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::Message;

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
    fn lossy_drops_older_messages() {
        let mut messages = vec![Message::system("sys")];
        for i in 0..20 {
            messages.push(Message::user(format!("msg {i} {}", "x".repeat(80))));
        }
        let lossy = prepare_summary_input(
            &messages,
            InputLadderStage::Lossy,
            Some(200),
            &TokenEstimator::default(),
        );
        assert!(lossy.len() < messages.len());
        assert_eq!(lossy[0].role, Role::System);
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
