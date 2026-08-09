use std::collections::HashMap;

use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use sha2::{Digest, Sha256};

use crate::message::Message;
use crate::openai_compatible::message_to_api;

/// LLM gateway session mode (Zene Session Protocol v0.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatewayMode {
    Publish,
    Delta,
    Full,
}

impl GatewayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Publish => "publish",
            Self::Delta => "delta",
            Self::Full => "full",
        }
    }
}

/// Side-channel LLM calls that must not append to the main session tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GatewaySubcall {
    #[default]
    Main,
    Compaction,
    Memory,
    Subagent,
}

impl GatewaySubcall {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Main => "main",
            Self::Compaction => "compaction",
            Self::Memory => "memory",
            Self::Subagent => "subagent",
        }
    }
}

/// Outbound gateway session context attached to a chat request.
#[derive(Debug, Clone)]
pub struct GatewayRequestContext {
    pub session_id: String,
    pub epoch: u32,
    pub mode: GatewayMode,
    pub prefix_hash: Option<String>,
    pub subcall: GatewaySubcall,
    pub prefix_len: Option<usize>,
}

impl GatewayRequestContext {
    pub fn http_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::from([
            (
                "x-zene-session-id".to_string(),
                self.session_id.clone(),
            ),
            (
                "x-zene-epoch".to_string(),
                self.epoch.to_string(),
            ),
            (
                "x-zene-mode".to_string(),
                self.mode.as_str().to_string(),
            ),
            (
                "x-zene-subcall".to_string(),
                self.subcall.as_str().to_string(),
            ),
        ]);
        if let Some(hash) = &self.prefix_hash {
            headers.insert("x-zene-prefix-hash".to_string(), hash.clone());
        }
        if let Some(len) = self.prefix_len {
            headers.insert("x-zene-prefix-len".to_string(), len.to_string());
        }
        headers
    }

    pub fn apply_reqwest_headers(&self, headers: &mut HeaderMap) {
        for (name, value) in self.http_headers() {
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(name.as_bytes()),
                HeaderValue::from_str(&value),
            ) {
                headers.insert(name, value);
            }
        }
    }
}

/// Stable SHA-256 hash over canonical prefix messages (OpenAI JSON, sorted keys).
pub fn prefix_hash(messages: &[Message]) -> anyhow::Result<String> {
    let array: Vec<_> = messages
        .iter()
        .map(message_to_api)
        .collect::<anyhow::Result<_>>()?;
    let json = serde_json::to_string(&array)?;
    let digest = Sha256::digest(json.as_bytes());
    Ok(format!(
        "sha256:{}",
        digest.iter().map(|b| format!("{b:02x}")).collect::<String>()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::Message;

    #[test]
    fn prefix_hash_is_stable() {
        let msgs = vec![
            Message::system("sys"),
            Message::user("hi"),
        ];
        let h1 = prefix_hash(&msgs).expect("hash");
        let h2 = prefix_hash(&msgs).expect("hash");
        assert_eq!(h1, h2);
        assert!(h1.starts_with("sha256:"));
    }

    #[test]
    fn gateway_headers_include_mode() {
        let ctx = GatewayRequestContext {
            session_id: "abc".into(),
            epoch: 2,
            mode: GatewayMode::Delta,
            prefix_hash: Some("sha256:dead".into()),
            subcall: GatewaySubcall::Main,
            prefix_len: None,
        };
        let h = ctx.http_headers();
        assert_eq!(h.get("x-zene-mode").map(String::as_str), Some("delta"));
        assert_eq!(h.get("x-zene-epoch").map(String::as_str), Some("2"));
    }
}
