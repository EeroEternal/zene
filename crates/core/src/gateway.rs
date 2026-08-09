//! LLM gateway session helpers (Zene Session Protocol v0.1).

use anyhow::Result;
use zene_config::{GatewaySessionConfig, ZeneConfig};
use zene_llm::{
    prefix_hash, ChatRequest, GatewayMode, GatewayRequestContext, GatewaySubcall, Message,
    MessageKind, Role, TokenUsage,
};
use zene_session::{GatewaySessionState, SessionRecord};

use crate::memory;

pub fn gateway_session_enabled(config: &ZeneConfig) -> bool {
    config.gateway_session.enabled
}

pub fn resolve_session_id(session_meta_id: &str) -> String {
    std::env::var("ZENE_SESSION_ID")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| session_meta_id.to_string())
}

pub fn is_canonical_prefix_message(msg: &Message) -> bool {
    if msg.role == Role::System {
        return true;
    }
    if msg.kind == Some(MessageKind::CompactionSummary) {
        return true;
    }
    if msg.content.as_deref().is_some_and(|c| c.contains(memory::MEMORY_CONTEXT_OPEN)) {
        return true;
    }
    false
}

pub fn canonical_prefix_len(messages: &[Message]) -> usize {
    let mut len = 0;
    for msg in messages {
        if is_canonical_prefix_message(msg) {
            len += 1;
        } else {
            break;
        }
    }
    len
}

pub fn ensure_gateway_state(session: &mut SessionRecord) {
    if session.gateway.is_none() {
        session.gateway = Some(GatewaySessionState::default());
    }
}

pub fn bump_gateway_epoch(session: &mut SessionRecord) {
    ensure_gateway_state(session);
    if let Some(gw) = session.gateway.as_mut() {
        gw.epoch = gw.epoch.saturating_add(1);
        gw.synced_message_count = 0;
        gw.prefix_len = None;
        gw.prefix_hash = None;
    }
}

pub fn mark_gateway_publish_needed(session: &mut SessionRecord) {
    bump_gateway_epoch(session);
}

pub fn record_gateway_usage(session: &mut SessionRecord, usage: &TokenUsage) {
    ensure_gateway_state(session);
    if let Some(gw) = session.gateway.as_mut() {
        gw.last_prompt_tokens = Some(usage.prompt_tokens);
        gw.last_cached_tokens = usage.cached_tokens;
    }
}

pub fn mark_gateway_synced(session: &mut SessionRecord, message_count: usize, prefix_len: usize) {
    ensure_gateway_state(session);
    if let Some(gw) = session.gateway.as_mut() {
        gw.synced_message_count = message_count;
        gw.prefix_len = Some(prefix_len);
    }
}

pub struct GatewayRequestPlan {
    pub messages: Vec<Message>,
    pub mode: GatewayMode,
    pub prefix_hash: Option<String>,
    pub prefix_len: Option<usize>,
}

pub fn plan_gateway_request(
    session: &SessionRecord,
    full_messages: &[Message],
    cfg: &GatewaySessionConfig,
    subcall: GatewaySubcall,
    publish_pending: bool,
) -> Result<Option<GatewayRequestPlan>> {
    if subcall != GatewaySubcall::Main {
        return Ok(Some(GatewayRequestPlan {
            messages: full_messages.to_vec(),
            mode: GatewayMode::Full,
            prefix_hash: None,
            prefix_len: None,
        }));
    }

    let gw = session.gateway.as_ref();
    let prefix_len = canonical_prefix_len(full_messages);
    let prefix: Vec<Message> = full_messages[..prefix_len].to_vec();
    let prefix_hash_val = if prefix.is_empty() {
        None
    } else {
        Some(prefix_hash(&prefix)?)
    };

    if publish_pending {
        return Ok(Some(GatewayRequestPlan {
            messages: prefix,
            mode: GatewayMode::Publish,
            prefix_hash: prefix_hash_val.clone(),
            prefix_len: Some(prefix_len),
        }));
    }

    let synced = gw.map(|g| g.synced_message_count).unwrap_or(0);
    if cfg.use_delta() && synced < full_messages.len() {
        let delta = full_messages[synced..].to_vec();
        if !delta.is_empty() {
            return Ok(Some(GatewayRequestPlan {
                messages: delta,
                mode: GatewayMode::Delta,
                prefix_hash: prefix_hash_val,
                prefix_len: Some(prefix_len),
            }));
        }
    }

    Ok(Some(GatewayRequestPlan {
        messages: full_messages.to_vec(),
        mode: GatewayMode::Full,
        prefix_hash: prefix_hash_val,
        prefix_len: Some(prefix_len),
    }))
}

pub fn attach_gateway_context(
    mut request: ChatRequest,
    session: &SessionRecord,
    plan: &GatewayRequestPlan,
    subcall: GatewaySubcall,
) -> ChatRequest {
    let epoch = session
        .gateway
        .as_ref()
        .map(|g| g.epoch)
        .unwrap_or(0);
    request.gateway = Some(GatewayRequestContext {
        session_id: resolve_session_id(&session.meta.id),
        epoch,
        mode: plan.mode,
        prefix_hash: plan.prefix_hash.clone(),
        subcall,
        prefix_len: plan.prefix_len,
    });
    request
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_prefix_stops_at_first_user_message() {
        let messages = vec![
            Message::system("sys"),
            Message::user("hello"),
            Message::assistant("hi"),
        ];
        assert_eq!(canonical_prefix_len(&messages), 1);
    }

    #[test]
    fn resolve_session_id_prefers_env() {
        std::env::set_var("ZENE_SESSION_ID", "run-123");
        assert_eq!(resolve_session_id("local-id"), "run-123");
        std::env::remove_var("ZENE_SESSION_ID");
    }
}
