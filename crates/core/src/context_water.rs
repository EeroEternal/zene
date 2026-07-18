//! Usage-driven context water level (aligned with grok's session signals).
//!
//! Prefers the last provider-reported prompt token count when available;
//! falls back to the heuristic estimator for pre-sample checks.

use zene_config::CompactionConfig;
use zene_llm::TokenUsage;

/// Live context occupancy used for auto-compact and UI status.
#[derive(Debug, Clone, Default)]
pub struct ContextWaterLevel {
    /// Last successful request `prompt_tokens` from the provider (if any).
    pub last_prompt_tokens: Option<u32>,
    /// Last heuristic estimate computed before a sample.
    pub last_estimate_tokens: Option<u32>,
    /// Configured context window for the active model.
    pub context_window_tokens: u32,
}

impl ContextWaterLevel {
    pub fn new(context_window_tokens: u32) -> Self {
        Self {
            last_prompt_tokens: None,
            last_estimate_tokens: None,
            context_window_tokens,
        }
    }

    pub fn set_window(&mut self, context_window_tokens: u32) {
        self.context_window_tokens = context_window_tokens;
    }

    pub fn record_estimate(&mut self, estimate: u32) {
        self.last_estimate_tokens = Some(estimate);
    }

    pub fn record_usage(&mut self, usage: &TokenUsage) {
        if usage.prompt_tokens > 0 {
            self.last_prompt_tokens = Some(usage.prompt_tokens.min(u64::from(u32::MAX)) as u32);
        }
    }

    /// Effective tokens for compaction trigger: prefer real usage, else estimate.
    pub fn effective_tokens(&self) -> u32 {
        match (self.last_prompt_tokens, self.last_estimate_tokens) {
            (Some(usage), Some(est)) => usage.max(est),
            (Some(usage), None) => usage,
            (None, Some(est)) => est,
            (None, None) => 0,
        }
    }

    /// Percentage of the context window used (0..=100).
    pub fn usage_percent(&self) -> u8 {
        let window = self.context_window_tokens.max(1);
        let used = self.effective_tokens() as u64;
        ((used * 100) / u64::from(window)).min(100) as u8
    }

    pub fn should_compact(&self, config: &CompactionConfig) -> bool {
        let threshold =
            (config.context_window_tokens as f32 * config.trigger_ratio).floor() as u32;
        self.effective_tokens() >= threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_usage_when_higher_than_estimate() {
        let mut water = ContextWaterLevel::new(1000);
        water.record_estimate(400);
        water.record_usage(&TokenUsage {
            prompt_tokens: 700,
            completion_tokens: 10,
            total_tokens: 710,
        });
        assert_eq!(water.effective_tokens(), 700);
        assert_eq!(water.usage_percent(), 70);
    }

    #[test]
    fn uses_estimate_when_no_usage() {
        let mut water = ContextWaterLevel::new(1000);
        water.record_estimate(850);
        assert!(water.should_compact(&CompactionConfig {
            trigger_ratio: 0.85,
            keep_recent_ratio: 0.25,
            context_window_tokens: 1000,
            min_keep_messages: 4,
        }));
    }
}
