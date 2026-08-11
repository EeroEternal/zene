//! Bridge `zene-config` compaction settings to the context engine.

use zene_config::CompactionConfig as ConfigCompaction;
use zene_context::CompactionConfig;

pub fn context_compaction_config(config: &ConfigCompaction) -> CompactionConfig {
    CompactionConfig {
        trigger_ratio: config.trigger_ratio,
        keep_recent_ratio: config.keep_recent_ratio,
        context_window_tokens: config.context_window_tokens,
        min_keep_messages: config.min_keep_messages,
        intra_steps_first: config.intra_steps_first,
    }
}
