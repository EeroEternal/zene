//! Runtime-agnostic session view for context operations.

use anyhow::Result;
use zene_llm::Message;
use zene_session::{
    save_checkpoint, ProjectionFallbackReason, SessionEvent, SessionRecord, SessionView,
};

/// Mutable conversation state consumed by [`ContextEngine`](crate::ContextEngine).
pub trait ContextSession: Send + Sync {
    fn session_id(&self) -> &str;
    fn messages(&self) -> &[Message];
    fn messages_mut(&mut self) -> &mut Vec<Message>;
    fn events(&self) -> &[SessionEvent] {
        &[]
    }
    fn view(&self) -> SessionView {
        SessionView::from_events_for_session(
            self.events(),
            self.messages(),
            Some(self.session_id()),
        )
    }
    /// Strict event-backed projection for new context/model requests.
    fn try_view(&self) -> std::result::Result<SessionView, ProjectionFallbackReason> {
        SessionView::try_from_events(self.events(), self.messages(), Some(self.session_id()))
    }
    fn compaction_cycle(&self) -> u64;
    fn update_context_usage(&mut self, tokens_used: u32, context_window: u32);
    fn ensure_system_message(&mut self, content: &str);
    fn persist_checkpoint(&mut self, reason: &str) -> Result<()> {
        let _ = reason;
        Ok(())
    }
    /// Commit a compaction projection as one conversation fact and refresh the
    /// compatibility cache at the session boundary. Context code must not
    /// mutate `messages` directly.
    fn commit_compaction_snapshot(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
        messages_after: Vec<Message>,
    );
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

    fn events(&self) -> &[SessionEvent] {
        SessionRecord::events(self)
    }

    fn view(&self) -> SessionView {
        SessionRecord::view(self)
    }

    fn try_view(&self) -> std::result::Result<SessionView, ProjectionFallbackReason> {
        SessionRecord::try_view(self)
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

    fn persist_checkpoint(&mut self, reason: &str) -> Result<()> {
        let checkpoint = save_checkpoint(self, reason)?;
        self.record_checkpoint(None, None, None, "context_checkpoint", &checkpoint.id);
        Ok(())
    }

    fn commit_compaction_snapshot(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
        messages_after: Vec<Message>,
    ) {
        SessionRecord::record_compaction_event_with_messages(
            self,
            reason,
            compacted_count,
            summary,
            tokens_before,
            tokens_after,
            Some(messages_after.clone()),
        );
        self.messages = messages_after;
    }

    fn record_compaction_event(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
    ) {
        self.commit_compaction_snapshot(
            reason,
            compacted_count,
            summary,
            tokens_before,
            tokens_after,
            self.messages.clone(),
        );
    }

    fn patch_last_compaction_tokens_after(&mut self, tokens_after: u32) {
        if let Some(entry) = self.compactions.last_mut() {
            entry.tokens_after = Some(tokens_after);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use std::path::Path;
    use zene_session::{CompactionEntry, ProjectionFallbackReason};

    #[test]
    fn strict_projection_rejects_legacy_compaction_without_snapshot() {
        let mut session = SessionRecord::new(Path::new("."));
        session.messages.push(Message::user("cached"));
        session.events.push(SessionEvent::CompactionApplied {
            sequence: 1,
            id: "legacy-compaction".into(),
            created_at: Utc::now(),
            entry: CompactionEntry {
                id: "compact".into(),
                created_at: Utc::now(),
                summary: "old summary".into(),
                compacted_message_count: 1,
                reason: None,
                tokens_before: None,
                tokens_after: None,
            },
            messages_after: None,
        });

        let error = <SessionRecord as ContextSession>::try_view(&session)
            .expect_err("legacy fallback must be explicit");
        assert_eq!(
            error,
            ProjectionFallbackReason::LegacyCompactionWithoutSnapshot
        );
    }
}
