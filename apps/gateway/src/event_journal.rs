use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Notify, RwLock};

const DEFAULT_MAX_EVENTS: usize = 10_000;
const DEFAULT_RETENTION: Duration = Duration::from_secs(30 * 60);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JournalEvent {
    pub cursor: u64,
    pub created_at: DateTime<Utc>,
    pub payload: Value,
}

#[derive(Debug, Clone)]
struct StoredEvent {
    event: JournalEvent,
    stored_at: Instant,
}

#[derive(Debug)]
struct JournalInner {
    next_cursor: u64,
    events: VecDeque<StoredEvent>,
    max_events: usize,
    retention: Duration,
}

impl JournalInner {
    fn prune(&mut self) {
        let cutoff = Instant::now()
            .checked_sub(self.retention)
            .unwrap_or_else(Instant::now);
        while self.events.len() > self.max_events {
            self.events.pop_front();
        }
        while self
            .events
            .front()
            .is_some_and(|event| event.stored_at < cutoff)
            && self.events.len() > 1
        {
            // Keep at least the newest event when pruning by age so nextCursor remains meaningful.
            if self.events.len() == 1 {
                break;
            }
            self.events.pop_front();
        }
    }

    fn oldest_cursor(&self) -> Option<u64> {
        self.events.front().map(|event| event.event.cursor)
    }

    fn latest_cursor(&self) -> u64 {
        self.next_cursor.saturating_sub(1)
    }
}

#[derive(Debug, Clone)]
pub struct EventJournal {
    inner: Arc<RwLock<JournalInner>>,
    notify: Arc<Notify>,
}

impl EventJournal {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(JournalInner {
                next_cursor: 1,
                events: VecDeque::new(),
                max_events: DEFAULT_MAX_EVENTS,
                retention: DEFAULT_RETENTION,
            })),
            notify: Arc::new(Notify::new()),
        }
    }

    pub async fn append(&self, payload: Value) -> u64 {
        let cursor = {
            let mut inner = self.inner.write().await;
            let cursor = inner.next_cursor;
            inner.next_cursor += 1;
            inner.events.push_back(StoredEvent {
                event: JournalEvent {
                    cursor,
                    created_at: Utc::now(),
                    payload,
                },
                stored_at: Instant::now(),
            });
            inner.prune();
            cursor
        };
        self.notify.notify_waiters();
        cursor
    }

    pub async fn append_system(&self, kind: &str, message: impl Into<String>) -> u64 {
        self.append(serde_json::json!({
            "type": "gateway.system",
            "kind": kind,
            "message": message.into(),
        }))
        .await
    }

    pub async fn snapshot_meta(&self) -> (Option<u64>, u64, usize) {
        let inner = self.inner.read().await;
        (
            inner.oldest_cursor(),
            inner.latest_cursor(),
            inner.events.len(),
        )
    }

    pub async fn read_after(
        &self,
        cursor: u64,
        limit: usize,
    ) -> Result<(Vec<JournalEvent>, u64, bool), CursorExpired> {
        let inner = self.inner.read().await;
        if let Some(oldest) = inner.oldest_cursor() {
            // Client missed events between cursor+1 and oldest-1.
            if oldest > cursor.saturating_add(1) {
                return Err(CursorExpired {
                    oldest_cursor: oldest,
                    latest_cursor: inner.latest_cursor(),
                });
            }
        }

        let mut events = Vec::new();
        for stored in &inner.events {
            if stored.event.cursor > cursor {
                events.push(stored.event.clone());
                if events.len() >= limit {
                    break;
                }
            }
        }
        let next_cursor = events
            .last()
            .map(|event| event.cursor)
            .unwrap_or(cursor);
        let has_more = inner
            .events
            .back()
            .is_some_and(|event| event.event.cursor > next_cursor);
        Ok((events, next_cursor, has_more))
    }

    pub async fn wait_for_events(
        &self,
        cursor: u64,
        limit: usize,
        wait: Duration,
    ) -> Result<(Vec<JournalEvent>, u64, bool), CursorExpired> {
        let (events, next_cursor, has_more) = self.read_after(cursor, limit).await?;
        if !events.is_empty() || wait.is_zero() {
            return Ok((events, next_cursor, has_more));
        }

        let notified = self.notify.notified();
        tokio::pin!(notified);
        let _ = tokio::time::timeout(wait, &mut notified).await;
        self.read_after(cursor, limit).await
    }
}

#[derive(Debug, Clone)]
pub struct CursorExpired {
    pub oldest_cursor: u64,
    pub latest_cursor: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn append_and_read_after_cursor() {
        let journal = EventJournal::new();
        let c1 = journal.append(json!({"n": 1})).await;
        let c2 = journal.append(json!({"n": 2})).await;
        assert_eq!(c1, 1);
        assert_eq!(c2, 2);

        let (events, next, has_more) = journal.read_after(0, 10).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(next, 2);
        assert!(!has_more);

        let (events, next, _) = journal.read_after(1, 10).await.unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].cursor, 2);
        assert_eq!(next, 2);
    }

    #[tokio::test]
    async fn wait_returns_when_event_arrives() {
        let journal = EventJournal::new();
        let journal2 = journal.clone();
        let handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            journal2.append(json!({"hello": true})).await;
        });
        let (events, _, _) = journal
            .wait_for_events(0, 10, Duration::from_secs(2))
            .await
            .unwrap();
        handle.await.unwrap();
        assert_eq!(events.len(), 1);
    }
}
