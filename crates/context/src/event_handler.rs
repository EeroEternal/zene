//! Runtime IO boundary for context events (checkpoint, segments, gateway, memory).

use anyhow::Result;
use async_trait::async_trait;

use crate::events::ContextEvent;
use crate::FlushResult;
use crate::segment_store::CompactionSegmentWrite;

/// Outcome of handling a single [`ContextEvent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventOutcome {
    Void,
    MemoryFlush(FlushResult),
}

/// Handles context side effects (persistence, gateway publish, memory flush).
///
/// The engine invokes this inline when an operation must complete before continuing
/// (e.g. memory flush before compaction). Implementations perform the actual IO.
#[async_trait]
pub trait ContextEventHandler: Send {
    async fn handle(&mut self, event: &ContextEvent) -> Result<EventOutcome>;

    /// Recent memory block for post-compaction reinjection.
    fn memory_reminder(&self) -> Option<String> {
        None
    }
}

/// No-op handler for tests and minimal integrations.
pub struct NoopContextEventHandler;

#[async_trait]
impl ContextEventHandler for NoopContextEventHandler {
    async fn handle(&mut self, _event: &ContextEvent) -> Result<EventOutcome> {
        Ok(EventOutcome::Void)
    }
}

/// Records events without performing IO (useful in unit tests).
pub struct RecordingContextEventHandler {
    pub events: Vec<ContextEvent>,
}

impl RecordingContextEventHandler {
    pub fn new() -> Self {
        Self {
            events: Vec::new(),
        }
    }
}

impl Default for RecordingContextEventHandler {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ContextEventHandler for RecordingContextEventHandler {
    async fn handle(&mut self, event: &ContextEvent) -> Result<EventOutcome> {
        self.events.push(event.clone());
        Ok(match event {
            ContextEvent::MemoryFlush { .. } => EventOutcome::MemoryFlush(FlushResult::NothingToStore),
            _ => EventOutcome::Void,
        })
    }
}

/// Convenience: persist a compaction segment write via handler.
pub async fn write_compaction_segment_via(
    handler: &mut dyn ContextEventHandler,
    write: CompactionSegmentWrite,
) -> Result<()> {
    handler
        .handle(&ContextEvent::CompactionSegment {
            session_id: write.session_id,
            body: write.body,
        })
        .await?;
    Ok(())
}
