//! Outbound message assembly: full history vs delta tail (Phase 3).

use tracing::warn;
use zene_llm::{Message, MessageKind, Role};

#[cfg(feature = "gateway")]
use crate::gateway::gateway_configured;
#[cfg(not(feature = "gateway"))]
use crate::gateway_stub::gateway_configured;
use crate::two_pass::fingerprint_messages;

const ENV_DELIVERY: &str = "ZENE_CONTEXT_DELIVERY";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeliveryMode {
    #[default]
    Full,
    Delta,
}

impl DeliveryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Delta => "delta",
        }
    }
}

/// Resolve delivery mode from env.
///
/// Delta requires the gateway to reconstruct the full prompt per session. The
/// protocol does not yet have capability negotiation (issue #128), so the default
/// stays Full even when a gateway is configured; delta must be opted into
/// explicitly via `ZENE_CONTEXT_DELIVERY=delta`.
pub fn delivery_mode_from_env() -> DeliveryMode {
    match std::env::var(ENV_DELIVERY)
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("delta") => {
            if !gateway_configured() {
                warn!(
                    env = ENV_DELIVERY,
                    "delta delivery requested but no inference gateway configured; \
                     tails will be sent as full prompts"
                );
            }
            DeliveryMode::Delta
        }
        Some("full") => DeliveryMode::Full,
        // Conservative default: a gateway that cannot rebuild full prompts would
        // silently forward incomplete deltas downstream.
        _ => DeliveryMode::Full,
    }
}

#[derive(Debug, Clone)]
pub struct AssembledOutbound {
    pub messages: Vec<Message>,
    pub mode: DeliveryMode,
    pub prefix_len: usize,
    pub prefix_hash: Option<String>,
    pub tail_start: Option<usize>,
}

/// Build outbound messages for one LLM step.
///
/// In delta mode, only messages after `gateway_prefix_len` are sent when the tail is
/// non-empty; otherwise falls back to full (e.g. immediately after publish).
pub fn assemble_outbound(
    messages: &[Message],
    gateway_prefix_len: usize,
    mode: DeliveryMode,
) -> AssembledOutbound {
    let prefix_len = gateway_prefix_len.min(messages.len());
    let prefix = &messages[..prefix_len];
    let prefix_hash = if prefix_len > 0 {
        Some(format!("{:016x}", fingerprint_messages(prefix)))
    } else {
        None
    };

    if mode == DeliveryMode::Full || prefix_len >= messages.len() {
        return AssembledOutbound {
            messages: messages.to_vec(),
            mode: DeliveryMode::Full,
            prefix_len,
            prefix_hash,
            tail_start: None,
        };
    }

    let tail = messages[prefix_len..].to_vec();
    if tail.is_empty() {
        return AssembledOutbound {
            messages: messages.to_vec(),
            mode: DeliveryMode::Full,
            prefix_len,
            prefix_hash,
            tail_start: None,
        };
    }

    AssembledOutbound {
        messages: tail,
        mode: DeliveryMode::Delta,
        prefix_len,
        prefix_hash,
        tail_start: Some(prefix_len),
    }
}

/// Index after the stable system prefix (system + pinned compaction summaries).
pub fn stable_system_boundary(messages: &[Message]) -> usize {
    let mut idx = 0usize;
    if messages.first().is_some_and(|m| m.role == Role::System) {
        idx = 1;
    }
    while idx < messages.len() {
        if messages[idx].kind == Some(MessageKind::CompactionSummary) {
            idx += 1;
        } else {
            break;
        }
    }
    idx
}

/// Message indices that begin a semantic anchor block, at or after `start`.
///
/// Anchors are turn starts (user messages) and tool-call group starts (assistant
/// messages carrying tool calls); their tool results belong to the same block.
/// Published to the inference gateway so it can score prefix liveness on
/// harness-declared boundaries instead of tokenizer heuristics (issue #128).
pub fn anchor_boundaries(messages: &[Message], start: usize) -> Vec<u64> {
    messages
        .iter()
        .enumerate()
        .skip(start)
        .filter(|(_, message)| match message.role {
            Role::User => true,
            Role::Assistant => {
                message.tool_calls.as_ref().is_some_and(|calls| !calls.is_empty())
            }
            _ => false,
        })
        .map(|(idx, _)| idx as u64)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use zene_llm::Message;

    #[test]
    fn full_mode_sends_all_messages() {
        let messages = vec![
            Message::system("sys"),
            Message::user("hi"),
            Message::assistant("hello"),
        ];
        let out = assemble_outbound(&messages, 0, DeliveryMode::Full);
        assert_eq!(out.mode, DeliveryMode::Full);
        assert_eq!(out.messages.len(), 3);
        assert!(out.tail_start.is_none());
    }

    #[test]
    fn delta_mode_sends_tail_only() {
        let messages = vec![
            Message::system("sys"),
            Message::user("old"),
            Message::assistant("a"),
            Message::user("new"),
        ];
        let out = assemble_outbound(&messages, 3, DeliveryMode::Delta);
        assert_eq!(out.mode, DeliveryMode::Delta);
        assert_eq!(out.messages.len(), 1);
        assert_eq!(out.messages[0].content.as_deref(), Some("new"));
        assert_eq!(out.tail_start, Some(3));
        assert!(out.prefix_hash.is_some());
    }

    #[test]
    fn delta_falls_back_to_full_when_tail_empty() {
        let messages = vec![Message::system("sys"), Message::user("only")];
        let out = assemble_outbound(&messages, 2, DeliveryMode::Delta);
        assert_eq!(out.mode, DeliveryMode::Full);
        assert_eq!(out.messages.len(), 2);
    }

    #[test]
    fn stable_system_boundary_includes_compaction_summaries() {
        let messages = vec![
            Message::system("sys"),
            Message::compaction_summary("summary 1"),
            Message::compaction_summary("summary 2"),
            Message::user("hi"),
        ];
        assert_eq!(stable_system_boundary(&messages), 3);
    }

    #[test]
    fn delivery_mode_defaults_to_full_without_explicit_opt_in() {
        // Issue #128: gateway presence alone must not enable delta — the gateway
        // may be unable to rebuild full prompts, which would silently truncate.
        std::env::remove_var(ENV_DELIVERY);
        assert_eq!(delivery_mode_from_env(), DeliveryMode::Full);
        std::env::set_var(ENV_DELIVERY, "delta");
        assert_eq!(delivery_mode_from_env(), DeliveryMode::Delta);
        std::env::set_var(ENV_DELIVERY, "full");
        assert_eq!(delivery_mode_from_env(), DeliveryMode::Full);
        std::env::remove_var(ENV_DELIVERY);
    }

    #[test]
    fn anchor_boundaries_mark_turns_and_tool_groups() {
        let messages = vec![
            Message::system("sys"),                          // 0 pinned prefix
            Message::user("turn 1"),                         // 1 anchor
            Message::assistant_with_tools(None, vec![]),      // 2 not an anchor (no calls)
            Message::user("turn 2"),                         // 3 anchor
            Message::assistant_with_tools(None, vec![zene_llm::ToolCall {
                id: "call-1".into(),
                name: "read".into(),
                arguments: "{}".into(),
            }]),                                              // 4 anchor (tool group)
            Message::tool_result("call-1", "read", "out"),      // 5 same block
            Message::assistant("done"),                      // 6 not an anchor
        ];
        assert_eq!(stable_system_boundary(&messages), 1);
        assert_eq!(anchor_boundaries(&messages, 1), vec![1, 3, 4]);
    }
}
