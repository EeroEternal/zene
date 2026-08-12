//! Default [`ContextEventHandler`] for [`Agent`](crate::Agent).

use std::path::Path;

use anyhow::Result;
use async_trait::async_trait;
use tracing::{info, warn};
use zene_context::{
    publish_prefix, CompactionSegmentStore, CompactionSegmentWrite, ContextEvent,
    ContextEventHandler, EventOutcome, FsCompactionSegmentStore, FsMemoryStore, run_memory_flush,
};
use zene_llm::ChatClient;

pub struct AgentContextHandler<'a> {
    client: &'a ChatClient,
    model: &'a str,
    segment_store: FsCompactionSegmentStore,
    memory_store: FsMemoryStore,
}

impl<'a> AgentContextHandler<'a> {
    pub fn new(client: &'a ChatClient, model: &'a str, workdir: &'a Path) -> Self {
        Self {
            client,
            model,
            segment_store: FsCompactionSegmentStore::new(),
            memory_store: FsMemoryStore::new(workdir),
        }
    }
}

#[async_trait]
impl ContextEventHandler for AgentContextHandler<'_> {
    async fn handle(&mut self, event: &ContextEvent) -> Result<EventOutcome> {
        match event {
            ContextEvent::Checkpoint { .. } | ContextEvent::EpochBumped { .. } => {
                Ok(EventOutcome::Void)
            }
            ContextEvent::CompactionSegment { session_id, body } => {
                let write = CompactionSegmentWrite {
                    session_id: session_id.clone(),
                    body: body.clone(),
                };
                match self.segment_store.write_segment(&write) {
                    Ok(path) => {
                        info!(path = %path.display(), "wrote compaction segment");
                    }
                    Err(err) => {
                        warn!(error = %err, "failed to write compaction segment");
                    }
                }
                Ok(EventOutcome::Void)
            }
            ContextEvent::PublishPrefix {
                session_id,
                epoch,
                messages,
            } => {
                publish_prefix(session_id, *epoch, messages).await;
                Ok(EventOutcome::Void)
            }
            ContextEvent::MemoryFlush { conversation } => {
                let result =
                    run_memory_flush(self.client, self.model, conversation, &self.memory_store)
                        .await?;
                Ok(EventOutcome::MemoryFlush(result))
            }
        }
    }

    fn memory_reminder(&self) -> Option<String> {
        zene_context::memory_reminder_from_store(&self.memory_store)
    }
}
