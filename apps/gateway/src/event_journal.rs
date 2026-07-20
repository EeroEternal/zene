use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Notify, RwLock};

const DEFAULT_MAX_EVENTS: usize = 10_000;
const DEFAULT_MAX_BYTES: usize = 32 * 1024 * 1024;
const DEFAULT_RETENTION: Duration = Duration::from_secs(30 * 60);
const DEFAULT_MAX_PAYLOAD_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    approx_bytes: usize,
}

#[derive(Debug)]
struct JournalInner {
    next_cursor: u64,
    events: VecDeque<StoredEvent>,
    max_events: usize,
    max_bytes: usize,
    max_payload_bytes: usize,
    retention: Duration,
    total_bytes: usize,
    persist_path: Option<PathBuf>,
}

impl JournalInner {
    fn prune(&mut self) {
        let cutoff = Instant::now()
            .checked_sub(self.retention)
            .unwrap_or_else(Instant::now);
        while self.events.len() > self.max_events || self.total_bytes > self.max_bytes {
            if let Some(removed) = self.events.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(removed.approx_bytes);
            } else {
                break;
            }
        }
        while self
            .events
            .front()
            .is_some_and(|event| event.stored_at < cutoff)
            && self.events.len() > 1
        {
            if let Some(removed) = self.events.pop_front() {
                self.total_bytes = self.total_bytes.saturating_sub(removed.approx_bytes);
            } else {
                break;
            }
        }
    }

    fn oldest_cursor(&self) -> Option<u64> {
        self.events.front().map(|event| event.event.cursor)
    }

    fn latest_cursor(&self) -> u64 {
        self.next_cursor.saturating_sub(1)
    }

    fn persist_event(&self, event: &JournalEvent) -> Result<()> {
        let Some(path) = &self.persist_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create journal dir {}", parent.display()))?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .with_context(|| format!("open journal {}", path.display()))?;
        serde_json::to_writer(&mut file, event)?;
        file.write_all(b"\n")?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct EventJournal {
    inner: Arc<RwLock<JournalInner>>,
    notify: Arc<Notify>,
}

impl EventJournal {
    pub fn new() -> Self {
        Self::with_limits(
            DEFAULT_MAX_EVENTS,
            DEFAULT_MAX_BYTES,
            DEFAULT_MAX_PAYLOAD_BYTES,
            None,
        )
    }

    pub fn with_persist(path: PathBuf) -> Self {
        Self::with_limits(
            DEFAULT_MAX_EVENTS,
            DEFAULT_MAX_BYTES,
            DEFAULT_MAX_PAYLOAD_BYTES,
            Some(path),
        )
    }

    pub fn with_limits(
        max_events: usize,
        max_bytes: usize,
        max_payload_bytes: usize,
        persist_path: Option<PathBuf>,
    ) -> Self {
        Self {
            inner: Arc::new(RwLock::new(JournalInner {
                next_cursor: 1,
                events: VecDeque::new(),
                max_events,
                max_bytes,
                max_payload_bytes,
                retention: DEFAULT_RETENTION,
                total_bytes: 0,
                persist_path,
            })),
            notify: Arc::new(Notify::new()),
        }
    }

    pub fn load_from_file(path: &Path) -> Result<Self> {
        let mut inner = JournalInner {
            next_cursor: 1,
            events: VecDeque::new(),
            max_events: DEFAULT_MAX_EVENTS,
            max_bytes: DEFAULT_MAX_BYTES,
            max_payload_bytes: DEFAULT_MAX_PAYLOAD_BYTES,
            retention: DEFAULT_RETENTION,
            total_bytes: 0,
            persist_path: Some(path.to_path_buf()),
        };
        if path.exists() {
            let file = File::open(path).with_context(|| format!("open {}", path.display()))?;
            let reader = BufReader::new(file);
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                let event: JournalEvent = serde_json::from_str(&line)
                    .with_context(|| format!("parse journal line in {}", path.display()))?;
                let approx_bytes = approx_event_bytes(&event);
                inner.next_cursor = inner.next_cursor.max(event.cursor.saturating_add(1));
                inner.total_bytes = inner.total_bytes.saturating_add(approx_bytes);
                inner.events.push_back(StoredEvent {
                    event,
                    stored_at: Instant::now(),
                    approx_bytes,
                });
            }
            inner.prune();
        }
        Ok(Self {
            inner: Arc::new(RwLock::new(inner)),
            notify: Arc::new(Notify::new()),
        })
    }

    pub async fn append(&self, payload: Value) -> u64 {
        let payload = truncate_payload(payload, {
            let inner = self.inner.read().await;
            inner.max_payload_bytes
        });
        let cursor = {
            let mut inner = self.inner.write().await;
            let cursor = inner.next_cursor;
            inner.next_cursor += 1;
            let event = JournalEvent {
                cursor,
                created_at: Utc::now(),
                payload,
            };
            if let Err(err) = inner.persist_event(&event) {
                tracing::warn!("journal persist failed: {err}");
            }
            let approx_bytes = approx_event_bytes(&event);
            inner.total_bytes = inner.total_bytes.saturating_add(approx_bytes);
            inner.events.push_back(StoredEvent {
                event,
                stored_at: Instant::now(),
                approx_bytes,
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

    pub async fn snapshot_meta(&self) -> (Option<u64>, u64, usize, usize) {
        let inner = self.inner.read().await;
        (
            inner.oldest_cursor(),
            inner.latest_cursor(),
            inner.events.len(),
            inner.total_bytes,
        )
    }

    pub async fn persist_path(&self) -> Option<PathBuf> {
        self.inner.read().await.persist_path.clone()
    }

    pub async fn read_after(
        &self,
        cursor: u64,
        limit: usize,
    ) -> Result<(Vec<JournalEvent>, u64, bool), CursorExpired> {
        let inner = self.inner.read().await;
        if let Some(oldest) = inner.oldest_cursor() {
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

fn approx_event_bytes(event: &JournalEvent) -> usize {
    serde_json::to_vec(event).map(|v| v.len()).unwrap_or(256)
}

fn truncate_payload(payload: Value, max_bytes: usize) -> Value {
    let Ok(raw) = serde_json::to_vec(&payload) else {
        return payload;
    };
    if raw.len() <= max_bytes {
        return payload;
    }
    serde_json::json!({
        "type": "gateway.system",
        "kind": "payload_truncated",
        "message": format!("original payload {} bytes exceeded limit {}", raw.len(), max_bytes),
        "originalType": payload.get("type").cloned().unwrap_or(Value::Null),
        "originalMethod": payload.get("method").cloned().unwrap_or(Value::Null),
    })
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
    use tempfile::tempdir;

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

    #[tokio::test]
    async fn persists_and_reloads() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("journal.jsonl");
        let journal = EventJournal::with_persist(path.clone());
        journal.append(json!({"n": 1})).await;
        journal.append(json!({"n": 2})).await;

        let reloaded = EventJournal::load_from_file(&path).unwrap();
        let (events, next, _) = reloaded.read_after(0, 10).await.unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(next, 2);
        assert_eq!(events[1].payload["n"], 2);
    }

    #[tokio::test]
    async fn truncates_huge_payloads() {
        let journal = EventJournal::with_limits(100, 1024 * 1024, 64, None);
        let huge = "x".repeat(10_000);
        journal.append(json!({ "blob": huge })).await;
        let (events, _, _) = journal.read_after(0, 10).await.unwrap();
        assert_eq!(events[0].payload["kind"], "payload_truncated");
    }
}
