//! Optional inference-gateway hooks (Phase 2+). No-op when env is unset.

use tracing::{debug, warn};
use zene_llm::Message;

use crate::assemble::{anchor_boundaries, stable_system_boundary};
use crate::two_pass::fingerprint_messages;

const ENV_RUN_ID: &str = "ZENE_RUN_ID";
const ENV_GATEWAY_URL: &str = "ZENE_INFERENCE_GATEWAY_URL";

/// Cloud run id (or other external session key) injected by the worker.
pub fn external_session_id_from_env() -> Option<String> {
    std::env::var(ENV_RUN_ID)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn gateway_base_url() -> Option<String> {
    std::env::var(ENV_GATEWAY_URL)
        .ok()
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
}

/// True when inference gateway hooks are enabled.
pub fn gateway_configured() -> bool {
    gateway_base_url().is_some()
}

/// Notify gateway that canonical prefix changed (`epoch++` after compact / system change).
pub async fn publish_prefix(session_id: &str, epoch: u64, messages: &[Message]) {
    let Some(base) = gateway_base_url() else {
        debug!(
            session_id,
            epoch,
            message_count = messages.len(),
            "inference gateway publish skipped (no ZENE_INFERENCE_GATEWAY_URL)"
        );
        return;
    };
    let url = format!("{base}/v1/zene/sessions/{session_id}/publish");
    let api_messages: Vec<serde_json::Value> =
        messages.iter().map(message_to_publish_json).collect();
    let pinned_boundary = stable_system_boundary(messages) as u64;
    let anchors = anchor_boundaries(messages, pinned_boundary as usize);
    let fingerprint_value = format!("{:016x}", fingerprint_messages(messages));
    let body = serde_json::json!({
        "epoch": epoch,
        "message_count": messages.len(),
        "messages": api_messages,
        "pinned_boundary": pinned_boundary,
        "anchor_boundaries": anchors,
        "fingerprint": {
            "algorithm": "zene-v1",
            "value": fingerprint_value,
        },
    });
    match reqwest::Client::new().post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() => {
            info_gateway("publish", session_id, epoch, resp.status().as_u16());
        }
        Ok(resp) => {
            warn!(
                session_id,
                epoch,
                status = resp.status().as_u16(),
                "inference gateway publish failed"
            );
        }
        Err(err) => {
            warn!(session_id, epoch, error = %err, "inference gateway publish error");
        }
    }
}

fn message_to_publish_json(message: &Message) -> serde_json::Value {
    use zene_llm::Role;
    let role = match message.role {
        Role::System => "system",
        Role::User => "user",
        Role::Assistant => "assistant",
        Role::Tool => "tool",
    };
    let mut obj = serde_json::Map::from_iter([(
        "role".to_string(),
        serde_json::Value::String(role.to_string()),
    )]);
    if let Some(content) = &message.content {
        obj.insert(
            "content".to_string(),
            serde_json::Value::String(content.clone()),
        );
    }
    if let Some(id) = &message.tool_call_id {
        obj.insert(
            "tool_call_id".to_string(),
            serde_json::Value::String(id.clone()),
        );
    }
    if let Some(name) = &message.name {
        obj.insert("name".to_string(), serde_json::Value::String(name.clone()));
    }
    serde_json::Value::Object(obj)
}

/// Release inference-side session state when an agent run ends.
pub async fn close_session(session_id: &str) {
    let Some(base) = gateway_base_url() else {
        debug!(
            session_id,
            "inference gateway close skipped (no ZENE_INFERENCE_GATEWAY_URL)"
        );
        return;
    };
    let url = format!("{base}/v1/zene/sessions/{session_id}");
    match reqwest::Client::new().delete(&url).send().await {
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 404 => {
            info_gateway("close", session_id, 0, resp.status().as_u16());
        }
        Ok(resp) => {
            warn!(
                session_id,
                status = resp.status().as_u16(),
                "inference gateway close failed"
            );
        }
        Err(err) => {
            warn!(session_id, error = %err, "inference gateway close error");
        }
    }
}

fn info_gateway(op: &str, session_id: &str, epoch: u64, status: u16) {
    tracing::info!(
        op,
        session_id,
        epoch,
        status,
        "inference gateway session hook"
    );
}
