//! Runtime-agnostic session view for context operations.

use zene_llm::Message;
use zene_session::SessionRecord;

/// Mutable conversation state consumed by [`ContextEngine`](crate::ContextEngine).
pub trait ContextSession: Send + Sync {
    fn session_id(&self) -> &str;
    fn messages(&self) -> &[Message];
    fn messages_mut(&mut self) -> &mut Vec<Message>;
    fn compaction_cycle(&self) -> u64;
    fn update_context_usage(&mut self, tokens_used: u32, context_window: u32);
    fn ensure_system_message(&mut self, content: &str);
    fn record_compaction_event(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
    );
    fn patch_last_compaction_tokens_after(&mut self, tokens_after: u32);
}

impl ContextSession for SessionRecord {
    fn session_id(&self) -> &str {
        &self.meta.id
    }

    fn messages(&self) -> &[Message] {
        &self.messages
    }

    fn messages_mut(&mut self) -> &mut Vec<Message> {
        &mut self.messages
    }

    fn compaction_cycle(&self) -> u64 {
        self.compactions.len() as u64
    }

    fn update_context_usage(&mut self, tokens_used: u32, context_window: u32) {
        SessionRecord::update_context_usage(self, tokens_used, context_window);
    }

    fn ensure_system_message(&mut self, content: &str) {
        SessionRecord::ensure_system_message(self, content);
    }

    fn record_compaction_event(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
    ) {
        SessionRecord::record_compaction_event(
            self,
            reason,
            compacted_count,
            summary,
            tokens_before,
            tokens_after,
        );
    }

    fn patch_last_compaction_tokens_after(&mut self, tokens_after: u32) {
        if let Some(entry) = self.compactions.last_mut() {
            entry.tokens_after = Some(tokens_after);
        }
    }
}
