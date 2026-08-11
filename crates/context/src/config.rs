use serde::{Deserialize, Serialize};

pub const DEFAULT_CONTEXT_WINDOW_TOKENS: u32 = 128_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CompactionConfig {
    #[serde(default = "default_compaction_trigger_ratio")]
    pub trigger_ratio: f32,
    #[serde(default = "default_keep_recent_ratio")]
    pub keep_recent_ratio: f32,
    #[serde(default = "default_context_window_tokens")]
    pub context_window_tokens: u32,
    #[serde(default = "default_min_keep_messages")]
    pub min_keep_messages: usize,
    /// Before full compact, aggressively truncate tool results in the current
    /// turn's steps (after the last user message) — grok Intra Steps-first lite.
    #[serde(default = "default_intra_steps_first")]
    pub intra_steps_first: bool,
}

fn default_compaction_trigger_ratio() -> f32 {
    0.85
}

fn default_keep_recent_ratio() -> f32 {
    0.25
}

fn default_context_window_tokens() -> u32 {
    DEFAULT_CONTEXT_WINDOW_TOKENS
}

fn default_min_keep_messages() -> usize {
    20
}

fn default_intra_steps_first() -> bool {
    true
}

impl Default for CompactionConfig {
    fn default() -> Self {
        Self {
            trigger_ratio: default_compaction_trigger_ratio(),
            keep_recent_ratio: default_keep_recent_ratio(),
            context_window_tokens: default_context_window_tokens(),
            min_keep_messages: default_min_keep_messages(),
            intra_steps_first: default_intra_steps_first(),
        }
    }
}
