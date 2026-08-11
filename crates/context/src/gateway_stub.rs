//! No-op gateway hooks when `gateway` feature is disabled.

use zene_llm::Message;

const ENV_RUN_ID: &str = "ZENE_RUN_ID";

pub fn external_session_id_from_env() -> Option<String> {
    std::env::var(ENV_RUN_ID)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn gateway_configured() -> bool {
    false
}

pub async fn publish_prefix(_session_id: &str, _epoch: u64, _messages: &[Message]) {}

pub async fn close_session(_session_id: &str) {}
