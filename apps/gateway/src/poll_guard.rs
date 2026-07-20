//! Limits concurrent long-poll / SSE consumers per agent.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct PollGuard {
    inner: Arc<Mutex<HashMap<String, usize>>>,
    max_per_agent: usize,
}

pub struct PollPermit {
    guard: PollGuard,
    agent_id: String,
}

impl PollGuard {
    pub fn new(max_per_agent: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
            max_per_agent: max_per_agent.max(1),
        }
    }

    pub async fn try_acquire(&self, agent_id: &str) -> Option<PollPermit> {
        let mut map = self.inner.lock().ok()?;
        let count = map.entry(agent_id.to_string()).or_insert(0);
        if *count >= self.max_per_agent {
            return None;
        }
        *count += 1;
        Some(PollPermit {
            guard: self.clone(),
            agent_id: agent_id.to_string(),
        })
    }
}

impl Drop for PollPermit {
    fn drop(&mut self) {
        let Ok(mut map) = self.guard.inner.lock() else {
            return;
        };
        if let Some(count) = map.get_mut(&self.agent_id) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                map.remove(&self.agent_id);
            }
        }
    }
}
