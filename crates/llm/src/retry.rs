use std::time::Duration;

use anyhow::Result;
use tokio::time::sleep;

pub const MAX_LLM_ATTEMPTS: u32 = 3;

const CONTEXT_OVERFLOW_KEYWORDS: &[&str] = &[
    "context length",
    "context_length",
    "maximum context",
    "max context",
    "token limit",
    "tokens exceed",
    "too many tokens",
    "input is too long",
    "request too large",
];

/// HTTP 400-style context overflow — do not retry; compaction should handle.
pub fn is_context_overflow(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    if !(lower.contains("400") || lower.contains("bad request") || lower.contains("invalid_request")) {
        return CONTEXT_OVERFLOW_KEYWORDS
            .iter()
            .any(|kw| lower.contains(kw));
    }
    CONTEXT_OVERFLOW_KEYWORDS
        .iter()
        .any(|kw| lower.contains(kw))
}

/// Retryable: transport failures, rate limits, server errors.
pub fn is_retryable(err: &str) -> bool {
    if is_context_overflow(err) {
        return false;
    }
    let lower = err.to_ascii_lowercase();
    if lower.contains("429")
        || lower.contains("too many requests")
        || lower.contains("rate limit")
    {
        return true;
    }
    if lower.contains("502")
        || lower.contains("503")
        || lower.contains("504")
        || lower.contains("500")
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

pub fn retry_delay(attempt: u32) -> Duration {
    let ms = 500u64.saturating_mul(1u64 << attempt.saturating_sub(1).min(4));
    Duration::from_millis(ms.min(8_000))
}

pub async fn with_llm_retry<T, F, Fut>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    for attempt in 1..=MAX_LLM_ATTEMPTS {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(err) => {
                let msg = err.to_string();
                if is_context_overflow(&msg) {
                    return Err(err);
                }
                if !is_retryable(&msg) || attempt >= MAX_LLM_ATTEMPTS {
                    return Err(err);
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
    }

    #[test]
    fn rate_limit_is_retryable() {
        let err = "HTTP 429 Too Many Requests";
        assert!(!is_context_overflow(err));
        assert!(is_retryable(err));
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
    fn connection_error_retryable() {
        assert!(is_retryable("reqwest::Error: connection timed out"));
    }

    #[test]
    fn backoff_grows() {
        assert!(retry_delay(1) < retry_delay(2));
    }
}
