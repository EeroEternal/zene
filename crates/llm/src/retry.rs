use std::time::Duration;

use anyhow::Result;
use tokio::time::sleep;

/// Default max retries for transient server/transport errors (~aligned with grok budget).
pub const MAX_LLM_ATTEMPTS: u32 = 5;
/// Rate-limit (429) retries are capped lower to avoid long waits.
pub const RATE_LIMIT_RETRY_THRESHOLD: u32 = 2;

const CONTEXT_OVERFLOW_KEYWORDS: &[&str] = &[
    "context length",
    "context_length",
    "context window",
    "maximum context",
    "max context",
    "token limit",
    "tokens exceed",
    "too many tokens",
    "input is too long",
    "prompt is too long",
    "request too large",
    "context overflow",
    "model_context_window_exceeded",
    "413",
    "payload too large",
    "request entity too large",
];

/// Error class for sampling decisions (retry / compact / fatal).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LlmErrorClass {
    ContextOverflow,
    RateLimited,
    Transient,
    EmptyResponse,
    Fatal,
}

/// Context overflow — do not retry at the transport layer; compaction should handle.
pub fn is_context_overflow(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    CONTEXT_OVERFLOW_KEYWORDS
        .iter()
        .any(|kw| lower.contains(kw))
}

fn is_rate_limited(lower: &str) -> bool {
    lower.contains("429") || lower.contains("too many requests") || lower.contains("rate limit")
}

fn is_auth_or_client_fatal(lower: &str) -> bool {
    lower.contains("401")
        || lower.contains("403")
        || lower.contains("404")
        || lower.contains("invalid_api_key")
        || lower.contains("authentication")
        || lower.contains("unauthorized")
        || (lower.contains("400") && !is_context_overflow(lower))
        || lower.contains("invalid_request")
        || lower.contains("invalid tool")
}

fn is_transient_transport(lower: &str) -> bool {
    if lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("500")
        || lower.contains("520")
        || lower.contains("internal server error")
        || lower.contains("bad gateway")
        || lower.contains("service unavailable")
        || lower.contains("gateway timeout")
    {
        return true;
    }
    lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("connect")
        || lower.contains("dns")
        || lower.contains("reqwest")
        || lower.contains("hyper")
        || lower.contains("broken pipe")
        || lower.contains("connection reset")
        || lower.contains("connection refused")
        || lower.contains("network")
}

fn is_empty_response(lower: &str) -> bool {
    lower.contains("empty response")
        || lower.contains("no content")
        || lower.contains("model returned no")
}

pub fn classify_llm_error(err: &str) -> LlmErrorClass {
    let lower = err.to_ascii_lowercase();
    if is_context_overflow(&lower) {
        return LlmErrorClass::ContextOverflow;
    }
    if is_empty_response(&lower) {
        return LlmErrorClass::EmptyResponse;
    }
    if is_rate_limited(&lower) {
        return LlmErrorClass::RateLimited;
    }
    if is_auth_or_client_fatal(&lower) {
        return LlmErrorClass::Fatal;
    }
    if is_transient_transport(&lower) {
        return LlmErrorClass::Transient;
    }
    LlmErrorClass::Fatal
}

/// Retryable: transport failures, rate limits, server errors, empty responses.
pub fn is_retryable(err: &str) -> bool {
    matches!(
        classify_llm_error(err),
        LlmErrorClass::RateLimited | LlmErrorClass::Transient | LlmErrorClass::EmptyResponse
    )
}

pub fn retry_delay(attempt: u32) -> Duration {
    let ms = 500u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(4));
    Duration::from_millis(ms.min(8_000))
}

pub async fn with_llm_retry<T, F, Fut>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut rate_limit_hits = 0u32;
    for attempt in 1..=MAX_LLM_ATTEMPTS {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                let msg = err.to_string();
                let class = classify_llm_error(&msg);
                match class {
                    LlmErrorClass::ContextOverflow | LlmErrorClass::Fatal => return Err(err),
                    LlmErrorClass::RateLimited => {
                        rate_limit_hits += 1;
                        if rate_limit_hits > RATE_LIMIT_RETRY_THRESHOLD
                            || attempt >= MAX_LLM_ATTEMPTS
                        {
                            return Err(err);
                        }
                    }
                    LlmErrorClass::Transient | LlmErrorClass::EmptyResponse => {
                        if attempt >= MAX_LLM_ATTEMPTS {
                            return Err(err);
                        }
                    }
                }
                sleep(retry_delay(attempt)).await;
            }
        }
    }
    unreachable!("loop returns on last attempt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_overflow_not_retryable() {
        let err = "400 bad request: context length exceeded for model";
        assert!(is_context_overflow(err));
        assert!(!is_retryable(err));
        assert_eq!(classify_llm_error(err), LlmErrorClass::ContextOverflow);
    }

    #[test]
    fn prompt_too_long_is_overflow() {
        assert!(is_context_overflow("prompt is too long"));
    }

    #[test]
    fn rate_limit_is_retryable() {
        let err = "HTTP 429 Too Many Requests";
        assert!(!is_context_overflow(err));
        assert!(is_retryable(err));
        assert_eq!(classify_llm_error(err), LlmErrorClass::RateLimited);
    }

    #[test]
    fn server_error_is_retryable() {
        assert!(is_retryable("upstream returned 503 Service Unavailable"));
    }

    #[test]
    fn client_error_without_overflow_not_auto_retry() {
        assert!(!is_retryable("400 invalid tool name"));
    }

    #[test]
    fn auth_error_not_retryable() {
        assert!(!is_retryable("401 unauthorized: invalid api key"));
    }

    #[test]
    fn connection_error_retryable() {
        assert!(is_retryable("reqwest::Error: connection timed out"));
    }

    #[test]
    fn empty_response_retryable() {
        assert!(is_retryable("empty response from model"));
        assert_eq!(
            classify_llm_error("empty response from model"),
            LlmErrorClass::EmptyResponse
        );
    }

    #[test]
    fn backoff_grows() {
        assert!(retry_delay(1) < retry_delay(2));
    }
}
