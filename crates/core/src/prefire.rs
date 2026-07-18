//! Background prefire state for two-pass compaction.

use std::sync::Arc;

use parking_lot::Mutex;
use tokio::task::JoinHandle;

use crate::two_pass::fingerprint_messages;
use zene_llm::Message;

/// Cached NOTE₁ from a completed prefire pass1.
#[derive(Debug, Clone)]
pub struct PrefireCache {
    pub note1: String,
    pub fingerprint: u64,
    pub split_idx: usize,
    /// Absolute index into session.messages where the pass1 prefix ended
    /// (including any leading system message offset handled by caller).
    pub prefix_end: usize,
}

pub struct PrefireState {
    inner: Arc<Mutex<PrefireInner>>,
}

#[derive(Default)]
struct PrefireInner {
    cache: Option<PrefireCache>,
    handle: Option<JoinHandle<Option<PrefireCache>>>,
    /// Fingerprint we already launched a job for (avoid duplicate spawns).
    launched_for: Option<u64>,
}

impl Default for PrefireState {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(PrefireInner::default())),
        }
    }
}

impl PrefireState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&self) {
        let mut g = self.inner.lock();
        if let Some(handle) = g.handle.take() {
            handle.abort();
        }
        g.cache = None;
        g.launched_for = None;
    }

    pub fn is_in_flight(&self) -> bool {
        self.inner.lock().handle.is_some()
    }

    pub fn has_cache(&self) -> bool {
        self.inner.lock().cache.is_some()
    }

    pub fn store(&self, cache: PrefireCache) {
        let mut g = self.inner.lock();
        g.cache = Some(cache);
        g.handle = None;
    }

    pub fn take_cache(&self) -> Option<PrefireCache> {
        self.inner.lock().cache.take()
    }

    pub fn peek_cache(&self) -> Option<PrefireCache> {
        self.inner.lock().cache.clone()
    }

    /// Return cached NOTE₁ if fingerprint still matches the pass1 slice of `body`.
    pub fn valid_cache_for(&self, body: &[Message]) -> Option<PrefireCache> {
        let cache = self.peek_cache()?;
        if cache.split_idx == 0 || cache.split_idx > body.len() {
            return None;
        }
        let fp = fingerprint_messages(&body[..cache.split_idx]);
        if cache.fingerprint == fp {
            Some(cache)
        } else {
            None
        }
    }

    pub fn already_launched_for(&self, fingerprint: u64) -> bool {
        self.inner.lock().launched_for == Some(fingerprint)
    }

    pub fn set_handle(&self, fingerprint: u64, handle: JoinHandle<Option<PrefireCache>>) {
        let mut g = self.inner.lock();
        if let Some(old) = g.handle.take() {
            old.abort();
        }
        g.launched_for = Some(fingerprint);
        g.handle = Some(handle);
    }

    /// Wait for in-flight prefire (if any) and store the result.
    pub async fn await_in_flight(&self) {
        let handle = { self.inner.lock().handle.take() };
        if let Some(handle) = handle {
            match handle.await {
                Ok(Some(cache)) => self.store(cache),
                Ok(None) => {
                    self.inner.lock().launched_for = None;
                }
                Err(_) => {
                    self.inner.lock().launched_for = None;
                }
            }
        }
    }
}

/// Default percentage points below auto-compact threshold to start prefire.
pub fn prefire_lead_percent() -> u8 {
    std::env::var("ZENE_PREFIRE_LEAD_PERCENT")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(10)
}
