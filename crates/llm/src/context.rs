use serde::{Deserialize, Serialize};

/// How much conversation history is sent on an LLM request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ContextDelivery {
    #[default]
    Full,
    Delta,
}

impl ContextDelivery {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Delta => "delta",
        }
    }
}

/// Outbound metadata for inference-layer session linkage (see docs/context-engine.md).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextMetadata {
    pub session_id: String,
    pub context_epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix_hash: Option<String>,
    #[serde(default)]
    pub delivery: ContextDelivery,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tail_start: Option<usize>,
}

impl ContextMetadata {
    pub fn new(session_id: impl Into<String>, context_epoch: u64) -> Self {
        Self {
            session_id: session_id.into(),
            context_epoch,
            prefix_hash: None,
            delivery: ContextDelivery::Full,
            tail_start: None,
        }
    }
}

/// Gateway-only body field (unigateway `gateway_fields`; legacy alias).
pub const BODY_ZENE_CONTEXT: &str = "_zene_context";
/// UniGateway session delivery hints (`unigateway-session` / `gateway_fields`).
pub const SESSION_GATEWAY_FIELD: &str = "_session_context";
pub const HEADER_SESSION_ID: &str = "X-Zene-Session-Id";
pub const HEADER_CONTEXT_EPOCH: &str = "X-Zene-Context-Epoch";
pub const HEADER_CONTEXT_DELIVERY: &str = "X-Zene-Context-Delivery";
pub const HEADER_TAIL_START: &str = "X-Zene-Tail-Start";
pub const HEADER_PREFIX_HASH: &str = "X-Zene-Prefix-Hash";
