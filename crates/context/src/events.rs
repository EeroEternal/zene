//! Context side-effect events emitted by [`ContextEngine`](crate::ContextEngine).

use zene_llm::Message;

/// Events emitted when context state changes (for runtime hooks / observability).
#[derive(Debug, Clone)]
pub enum ContextEvent {
    EpochBumped {
        old: u64,
        new: u64,
        reason: &'static str,
    },
    /// Runtime should publish canonical prefix to inference gateway.
    PublishPrefix {
        session_id: String,
        epoch: u64,
        messages: Vec<Message>,
    },
    /// Runtime should persist a rewind checkpoint (e.g. before/after compact).
    Checkpoint { reason: &'static str },
    /// Runtime should persist a compaction segment for recovery.
    CompactionSegment { session_id: String, body: String },
    /// Runtime should run memory flush LLM + persist before compaction.
    MemoryFlush { conversation: String },
}
