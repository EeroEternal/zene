use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// Provider-reported prompt cache hits (OpenAI `prompt_tokens_details.cached_tokens`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<u64>,
    /// Inference-gateway/ledger-reported cache hit tokens (e.g. Cortex
    /// `usage.cache_hit_tokens`, mirrors the `x-cortex-cache-hit-tokens` header).
    /// Kept separate from `cached_tokens` so ledger view vs engine reality can be compared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_hit_tokens: Option<u64>,
}

impl TokenUsage {
    pub fn accumulate(&mut self, other: &TokenUsage) {
        self.prompt_tokens += other.prompt_tokens;
        self.completion_tokens += other.completion_tokens;
        self.total_tokens += other.total_tokens;
        match (self.cached_tokens, other.cached_tokens) {
            (Some(a), Some(b)) => self.cached_tokens = Some(a + b),
            (None, Some(b)) => self.cached_tokens = Some(b),
            _ => {}
        }
        match (self.gateway_hit_tokens, other.gateway_hit_tokens) {
            (Some(a), Some(b)) => self.gateway_hit_tokens = Some(a + b),
            (None, Some(b)) => self.gateway_hit_tokens = Some(b),
            _ => {}
        }
    }
}

pub fn parse_usage_from_raw(raw: &serde_json::Value) -> Option<TokenUsage> {
    let usage = raw.get("usage")?;
    let prompt_tokens = usage
        .get("prompt_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let completion_tokens = usage
        .get("completion_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    let total_tokens = usage
        .get("total_tokens")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(prompt_tokens + completion_tokens);

    let cached_tokens = usage
        .get("prompt_tokens_details")
        .and_then(|d| d.get("cached_tokens"))
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            usage
                .get("cached_tokens")
                .and_then(serde_json::Value::as_u64)
        });

    // Gateway-injected ledger hits (unigateway normalizes heterogeneous upstream
    // shapes into `usage.cache_hit_tokens`).
    let gateway_hit_tokens = usage
        .get("cache_hit_tokens")
        .and_then(serde_json::Value::as_u64);

    Some(TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens,
        gateway_hit_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_openai_usage() {
        let raw = json!({
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15
            }
        });
        let usage = parse_usage_from_raw(&raw).expect("usage");
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 5);
        assert_eq!(usage.total_tokens, 15);
    }

    #[test]
    fn parses_gateway_cache_hit_tokens() {
        // unigateway normalizes gateway/ledger hits into `usage.cache_hit_tokens`
        // (Cortex: mirrors the x-cortex-cache-hit-tokens header).
        let raw = json!({
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 5,
                "total_tokens": 105,
                "prompt_tokens_details": {"cached_tokens": 60},
                "cache_hit_tokens": 80
            }
        });
        let usage = parse_usage_from_raw(&raw).expect("usage");
        assert_eq!(usage.cached_tokens, Some(60));
        assert_eq!(usage.gateway_hit_tokens, Some(80));
    }

    #[test]
    fn accumulate_sums_fields() {
        let mut total = TokenUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: None,
            gateway_hit_tokens: None,
        };
        total.accumulate(&TokenUsage {
            prompt_tokens: 4,
            completion_tokens: 5,
            total_tokens: 9,
            cached_tokens: Some(2),
            gateway_hit_tokens: Some(7),
        });
        assert_eq!(total.prompt_tokens, 5);
        assert_eq!(total.completion_tokens, 7);
        assert_eq!(total.total_tokens, 12);
        assert_eq!(total.cached_tokens, Some(2));
        assert_eq!(total.gateway_hit_tokens, Some(7));
    }
}
