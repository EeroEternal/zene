//! No-op prefire when `prefire` feature is disabled.

use zene_llm::Message;

#[derive(Debug, Clone)]
pub struct PrefireCache {
    pub note1: String,
    pub fingerprint: u64,
    pub split_idx: usize,
}

pub struct PrefireState;

impl Default for PrefireState {
    fn default() -> Self {
        Self
    }
}

impl PrefireState {
    pub fn new() -> Self {
        Self
    }

    pub fn clear(&self) {}

    pub fn is_in_flight(&self) -> bool {
        false
    }

    pub fn has_cache(&self) -> bool {
        false
    }

    pub fn valid_cache_for(&self, _body: &[Message]) -> Option<PrefireCache> {
        None
    }

    pub fn already_launched_for(&self, _fingerprint: u64) -> bool {
        false
    }

    pub fn set_handle(
        &self,
        _fingerprint: u64,
        _handle: tokio::task::JoinHandle<Option<PrefireCache>>,
    ) {
    }

    pub async fn await_in_flight(&self) {}
}

pub fn prefire_lead_percent() -> u8 {
    0
}
