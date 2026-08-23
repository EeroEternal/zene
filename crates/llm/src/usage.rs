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
    /// Gateway-reported semantic-anchor alignment of the served prefix
    /// (Cortex `usage.gateway_anchor_aligned`). `true` means the exact match's
    /// final page boundary coincided with a structural block boundary, so the
    /// hit is likely to survive the next agentic context edit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gateway_anchor_aligned: Option<bool>,
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
        // Alignment is a per-step property, not a sum: last observed wins.
        if other.gateway_anchor_aligned.is_some() {
            self.gateway_anchor_aligned = other.gateway_anchor_aligned;
        }
    }
}

/// Locates the `usage` object in either a completed JSON body (`{usage: ...}`)
/// or a streaming aggregation (`[{...}, {usage: ...}]` — unigateway-sdk stores
/// the raw SSE events as an array on `ChatResponseFinal.raw`).
fn usage_object(raw: &serde_json::Value) -> Option<&serde_json::Value> {
    if let Some(usage) = raw.get("usage") {
        return Some(usage);
    }
    raw.as_array()?
        .iter()
        .rev()
        .find_map(|event| event.get("usage"))
}

/// Header names for Cortex gateway routing telemetry.
const HEADER_HIT_TOKENS: &str = "x-cortex-cache-hit-tokens";
const HEADER_ANCHOR_ALIGNED: &str = "x-cortex-anchor-aligned";

/// Merges gateway routing headers into parsed usage as a fallback for fields
/// the response body did not carry. Body values (per-request, injected by the
/// gateway) win over headers when both are present.
///
/// Source: unigateway response_headers (surfaced in unigateway#6); Cortex
/// emits `x-cortex-cache-hit-tokens` / `x-cortex-anchor-aligned`.
pub fn apply_gateway_headers(
    usage: &mut Option<TokenUsage>,
    headers: &std::collections::HashMap<String, String>,
) {
    let hit = headers
        .get(HEADER_HIT_TOKENS)
        .and_then(|v| v.parse::<u64>().ok());
    let aligned = headers
        .get(HEADER_ANCHOR_ALIGNED)
        .map(|v| v.trim() == "true");
    if hit.is_none() && aligned.is_none() {
        return;
    }
    match usage {
        Some(u) => {
            if u.gateway_hit_tokens.is_none() {
                u.gateway_hit_tokens = hit;
            }
            if u.gateway_anchor_aligned.is_none() {
                u.gateway_anchor_aligned = aligned;
            }
        }
        slot @ None => {
            *slot = Some(TokenUsage {
                gateway_hit_tokens: hit,
                gateway_anchor_aligned: aligned,
                ..TokenUsage::default()
            });
        }
    }
}

pub fn parse_usage_from_raw(raw: &serde_json::Value) -> Option<TokenUsage> {
    let usage = usage_object(raw)?;
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

    // Gateway-injected ledger hits. Two shapes exist in the wild:
    // - unigateway normalization: `usage.cache_hit_tokens`
    // - Cortex direct injection:  `usage.gateway_cache_hit_tokens`
    let gateway_hit_tokens = usage
        .get("gateway_cache_hit_tokens")
        .and_then(serde_json::Value::as_u64)
        .or_else(|| {
            usage
                .get("cache_hit_tokens")
                .and_then(serde_json::Value::as_u64)
        });

    let gateway_anchor_aligned = usage
        .get("gateway_anchor_aligned")
        .and_then(serde_json::Value::as_bool)
        .or_else(|| {
            usage
                .get("anchor_aligned")
                .and_then(serde_json::Value::as_bool)
        });

    Some(TokenUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens,
        gateway_hit_tokens,
        gateway_anchor_aligned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gateway_headers_fill_missing_fields_without_overriding_body() {
        use std::collections::HashMap;
        let mut headers = HashMap::new();
        headers.insert("x-cortex-cache-hit-tokens".to_string(), "320".to_string());
        headers.insert("x-cortex-anchor-aligned".to_string(), "true".to_string());

        // Body empty: headers create the usage entry.
        let mut none: Option<TokenUsage> = None;
        apply_gateway_headers(&mut none, &headers);
        let u = none.expect("created");
        assert_eq!(u.gateway_hit_tokens, Some(320));
        assert_eq!(u.gateway_anchor_aligned, Some(true));

        // Body present but fields missing: headers fill them.
        let mut some = Some(TokenUsage {
            prompt_tokens: 10,
            ..TokenUsage::default()
        });
        apply_gateway_headers(&mut some, &headers);
        let u = some.expect("kept");
        assert_eq!(u.gateway_hit_tokens, Some(320));
        assert_eq!(u.prompt_tokens, 10);

        // Body values win over headers when both exist.
        let mut both = Some(TokenUsage {
            gateway_hit_tokens: Some(64),
            gateway_anchor_aligned: Some(false),
            ..TokenUsage::default()
        });
        apply_gateway_headers(&mut both, &headers);
        let u = both.expect("kept");
        assert_eq!(u.gateway_hit_tokens, Some(64));
        assert_eq!(u.gateway_anchor_aligned, Some(false));

        // Unrelated headers are a no-op.
        let empty = HashMap::new();
        let mut none2: Option<TokenUsage> = None;
        apply_gateway_headers(&mut none2, &empty);
        assert!(none2.is_none());
    }

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
    fn parses_cortex_injected_fields() {
        // Cortex injects into the response body (unigateway-sdk's proxy_chat
        // does not surface response headers): usage.gateway_cache_hit_tokens +
        // usage.gateway_anchor_aligned.
        let raw = json!({
            "usage": {
                "prompt_tokens": 500,
                "completion_tokens": 16,
                "total_tokens": 516,
                "gateway_cache_hit_tokens": 320,
                "gateway_anchor_aligned": true
            }
        });
        let usage = parse_usage_from_raw(&raw).expect("usage");
        assert_eq!(usage.gateway_hit_tokens, Some(320));
        assert_eq!(usage.gateway_anchor_aligned, Some(true));
    }

    #[test]
    fn parses_usage_from_streaming_event_array() {
        // unigateway-sdk streaming completion stores raw as an array of SSE
        // payloads; the last event carries the usage object.
        let raw = json!([
            {"choices": [{"delta": {"content": "hi"}}]},
            {
                "choices": [],
                "usage": {
                    "prompt_tokens": 80,
                    "completion_tokens": 4,
                    "total_tokens": 84,
                    "gateway_cache_hit_tokens": 64,
                    "gateway_anchor_aligned": false
                }
            }
        ]);
        let usage = parse_usage_from_raw(&raw).expect("usage");
        assert_eq!(usage.prompt_tokens, 80);
        assert_eq!(usage.gateway_hit_tokens, Some(64));
        assert_eq!(usage.gateway_anchor_aligned, Some(false));
    }

    #[test]
    fn accumulate_sums_fields() {
        let mut total = TokenUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: None,
            gateway_hit_tokens: None,
            gateway_anchor_aligned: None,
        };
        total.accumulate(&TokenUsage {
            prompt_tokens: 4,
            completion_tokens: 5,
            total_tokens: 9,
            cached_tokens: Some(2),
            gateway_hit_tokens: Some(7),
            gateway_anchor_aligned: Some(true),
        });
        assert_eq!(total.prompt_tokens, 5);
        assert_eq!(total.completion_tokens, 7);
        assert_eq!(total.total_tokens, 12);
        assert_eq!(total.cached_tokens, Some(2));
        assert_eq!(total.gateway_hit_tokens, Some(7));
        // Alignment is a per-step property: last observed wins.
        assert_eq!(total.gateway_anchor_aligned, Some(true));
    }
}
