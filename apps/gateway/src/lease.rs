use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

const DEFAULT_TTL: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LeaseInfo {
    pub client_id: String,
    pub expires_in_ms: u64,
    pub held: bool,
}

#[derive(Debug, Clone)]
struct LeaseRecord {
    client_id: String,
    expires_at: Instant,
}

#[derive(Debug, Clone, Default)]
pub struct LeaseManager {
    inner: Arc<Mutex<HashMap<String, LeaseRecord>>>,
}

impl LeaseManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn acquire(
        &self,
        agent_id: &str,
        client_id: Option<String>,
        force: bool,
    ) -> Result<LeaseInfo, LeaseError> {
        let client_id = client_id
            .filter(|id| !id.trim().is_empty())
            .unwrap_or_else(|| format!("client_{}", Uuid::new_v4().simple()));
        let mut map = self.inner.lock().await;
        self.purge_expired(&mut map);
        if let Some(existing) = map.get(agent_id) {
            if existing.client_id != client_id && !force {
                return Err(LeaseError::HeldBy {
                    client_id: existing.client_id.clone(),
                    expires_in_ms: remaining_ms(existing.expires_at),
                });
            }
        }
        map.insert(
            agent_id.to_string(),
            LeaseRecord {
                client_id: client_id.clone(),
                expires_at: Instant::now() + DEFAULT_TTL,
            },
        );
        Ok(LeaseInfo {
            client_id,
            expires_in_ms: DEFAULT_TTL.as_millis() as u64,
            held: true,
        })
    }

    pub async fn heartbeat(
        &self,
        agent_id: &str,
        client_id: &str,
    ) -> Result<LeaseInfo, LeaseError> {
        let mut map = self.inner.lock().await;
        self.purge_expired(&mut map);
        match map.get_mut(agent_id) {
            Some(record) if record.client_id == client_id => {
                record.expires_at = Instant::now() + DEFAULT_TTL;
                Ok(LeaseInfo {
                    client_id: client_id.to_string(),
                    expires_in_ms: DEFAULT_TTL.as_millis() as u64,
                    held: true,
                })
            }
            Some(record) => Err(LeaseError::HeldBy {
                client_id: record.client_id.clone(),
                expires_in_ms: remaining_ms(record.expires_at),
            }),
            None => Err(LeaseError::NotHeld),
        }
    }

    pub async fn release(&self, agent_id: &str, client_id: &str) -> Result<(), LeaseError> {
        let mut map = self.inner.lock().await;
        self.purge_expired(&mut map);
        match map.get(agent_id) {
            Some(record) if record.client_id == client_id => {
                map.remove(agent_id);
                Ok(())
            }
            Some(record) => Err(LeaseError::HeldBy {
                client_id: record.client_id.clone(),
                expires_in_ms: remaining_ms(record.expires_at),
            }),
            None => Ok(()),
        }
    }

    pub async fn status(&self, agent_id: &str) -> Option<LeaseInfo> {
        let mut map = self.inner.lock().await;
        self.purge_expired(&mut map);
        map.get(agent_id).map(|record| LeaseInfo {
            client_id: record.client_id.clone(),
            expires_in_ms: remaining_ms(record.expires_at),
            held: true,
        })
    }

    /// Returns Ok when writes are allowed.
    /// No active lease ⇒ open (phase A compatibility).
    /// Active lease ⇒ only that client may write.
    pub async fn authorize_write(
        &self,
        agent_id: &str,
        client_id: Option<&str>,
    ) -> Result<(), LeaseError> {
        let mut map = self.inner.lock().await;
        self.purge_expired(&mut map);
        match map.get(agent_id) {
            None => Ok(()),
            Some(record) => match client_id {
                Some(id) if id == record.client_id => Ok(()),
                _ => Err(LeaseError::HeldBy {
                    client_id: record.client_id.clone(),
                    expires_in_ms: remaining_ms(record.expires_at),
                }),
            },
        }
    }

    fn purge_expired(&self, map: &mut HashMap<String, LeaseRecord>) {
        let now = Instant::now();
        map.retain(|_, record| record.expires_at > now);
    }
}

#[derive(Debug, Clone)]
pub enum LeaseError {
    HeldBy { client_id: String, expires_in_ms: u64 },
    NotHeld,
}

fn remaining_ms(expires_at: Instant) -> u64 {
    expires_at
        .saturating_duration_since(Instant::now())
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn force_acquire_steals_lease() {
        let leases = LeaseManager::new();
        let first = leases.acquire("a1", Some("c1".into()), false).await.unwrap();
        assert_eq!(first.client_id, "c1");
        let err = leases
            .acquire("a1", Some("c2".into()), false)
            .await
            .unwrap_err();
        assert!(matches!(err, LeaseError::HeldBy { .. }));
        let stolen = leases.acquire("a1", Some("c2".into()), true).await.unwrap();
        assert_eq!(stolen.client_id, "c2");
    }
}
