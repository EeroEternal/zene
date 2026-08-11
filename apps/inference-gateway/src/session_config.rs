//! Session store and middleware configuration from environment.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use unigateway_session::{
    FingerprintPolicy, MemorySessionStore, SessionLifetime, SessionMiddlewareConfig,
    SessionSizeLimits, SessionStore, SessionStoreConfig, TailPositionPolicy,
};
use unigateway_session_redis::{RedisSessionStore, RedisSessionStoreConfig};

pub const ENV_SESSION_REDIS_URL: &str = "ZENE_SESSION_REDIS_URL";
const ENV_FINGERPRINT_POLICY: &str = "ZENE_SESSION_FINGERPRINT_POLICY";
const ENV_IDLE_TTL_SECS: &str = "ZENE_SESSION_IDLE_TTL_SECS";
const ENV_MAX_LIFETIME_SECS: &str = "ZENE_SESSION_MAX_LIFETIME_SECS";
const ENV_MAX_PREFIX_BYTES: &str = "ZENE_SESSION_MAX_PREFIX_BYTES";
const ENV_MAX_TAIL_BYTES: &str = "ZENE_SESSION_MAX_TAIL_BYTES";
const ENV_MAX_ASSEMBLED_BYTES: &str = "ZENE_SESSION_MAX_ASSEMBLED_BYTES";
const ENV_REDIS_KEY_PREFIX: &str = "ZENE_SESSION_REDIS_KEY_PREFIX";
const ENV_PURGE_INTERVAL_SECS: &str = "ZENE_SESSION_PURGE_INTERVAL_SECS";

const DEFAULT_MAX_PREFIX_BYTES: usize = 4 * 1024 * 1024;
const DEFAULT_MAX_TAIL_BYTES: usize = 1024 * 1024;
const DEFAULT_MAX_ASSEMBLED_BYTES: usize = 8 * 1024 * 1024;
const DEFAULT_IDLE_TTL_SECS: u64 = 3600;
const DEFAULT_MAX_LIFETIME_SECS: u64 = 86_400;
const DEFAULT_PURGE_INTERVAL_SECS: u64 = 300;
const DEFAULT_REDIS_KEY_PREFIX: &str = "zene:session:";

#[derive(Debug, Clone)]
pub struct SessionRuntimeConfig {
    pub using_redis: bool,
    pub fingerprint_policy: FingerprintPolicy,
    pub tail_position_policy: TailPositionPolicy,
    pub size_limits: SessionSizeLimits,
    pub lifetime: SessionLifetime,
    pub touch_on_delta: bool,
    pub redis_key_prefix: String,
    pub purge_interval: Option<Duration>,
}

impl SessionRuntimeConfig {
    pub fn from_env() -> Self {
        let using_redis = redis_url_from_env().is_some();
        Self {
            using_redis,
            fingerprint_policy: fingerprint_policy_from_env(using_redis),
            tail_position_policy: TailPositionPolicy::Optional,
            size_limits: SessionSizeLimits {
                max_messages: None,
                max_prefix_bytes: Some(env_usize(ENV_MAX_PREFIX_BYTES, DEFAULT_MAX_PREFIX_BYTES)),
                max_tail_bytes: Some(env_usize(ENV_MAX_TAIL_BYTES, DEFAULT_MAX_TAIL_BYTES)),
                max_assembled_bytes: Some(env_usize(
                    ENV_MAX_ASSEMBLED_BYTES,
                    DEFAULT_MAX_ASSEMBLED_BYTES,
                )),
            },
            lifetime: SessionLifetime {
                idle_ttl: idle_ttl_from_env(using_redis),
                max_lifetime: max_lifetime_from_env(using_redis),
                touch_on_read: true,
            },
            touch_on_delta: true,
            redis_key_prefix: std::env::var(ENV_REDIS_KEY_PREFIX)
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| DEFAULT_REDIS_KEY_PREFIX.to_string()),
            purge_interval: purge_interval_from_env(using_redis),
        }
    }

    pub fn middleware_config(
        &self,
        key_resolver: unigateway_session::SessionKeyResolver,
    ) -> SessionMiddlewareConfig {
        SessionMiddlewareConfig {
            tail_position_policy: self.tail_position_policy,
            fingerprint_policy: self.fingerprint_policy,
            size_limits: self.size_limits,
            touch_on_delta: self.touch_on_delta,
            lifecycle_hook: None,
            key_resolver,
        }
    }

    fn store_config(&self) -> SessionStoreConfig {
        SessionStoreConfig {
            size_limits: self.size_limits,
            lifetime: self.lifetime,
            lifecycle_hook: None,
        }
    }
}

pub fn redis_url_from_env() -> Option<String> {
    std::env::var(ENV_SESSION_REDIS_URL)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

pub fn open_session_store(config: &SessionRuntimeConfig) -> anyhow::Result<Arc<dyn SessionStore>> {
    if config.using_redis {
        let url = redis_url_from_env().with_context(|| {
            format!("{ENV_SESSION_REDIS_URL} must be set when using Redis session store")
        })?;
        let store = RedisSessionStore::with_config(
            &url,
            RedisSessionStoreConfig {
                key_prefix: config.redis_key_prefix.clone(),
                size_limits: config.size_limits,
                lifetime: config.lifetime,
                lifecycle_hook: None,
            },
        )
        .with_context(|| format!("open Redis session store at {url}"))?;
        tracing::info!(redis_url = %url, "session store: redis");
        return Ok(Arc::new(store));
    }
    tracing::info!(
        "session store: in-memory (set {ENV_SESSION_REDIS_URL} for shared/production store)"
    );
    Ok(Arc::new(MemorySessionStore::with_config(
        config.store_config(),
    )))
}

pub fn spawn_purge_task(store: Arc<dyn SessionStore>, interval: Duration) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match store.purge_expired() {
                Ok(n) if n > 0 => tracing::info!(purged = n, "session purge"),
                Ok(_) => {}
                Err(err) => tracing::warn!(error = %err, "session purge failed"),
            }
        }
    });
}

fn fingerprint_policy_from_env(using_redis: bool) -> FingerprintPolicy {
    match std::env::var(ENV_FINGERPRINT_POLICY)
        .ok()
        .map(|s| s.trim().to_ascii_lowercase())
        .as_deref()
    {
        Some("required") => FingerprintPolicy::Required,
        Some("optional") => FingerprintPolicy::Optional,
        Some("disabled") | Some("off") => FingerprintPolicy::Disabled,
        _ if using_redis => FingerprintPolicy::Required,
        _ => FingerprintPolicy::Optional,
    }
}

fn idle_ttl_from_env(using_redis: bool) -> Option<Duration> {
    if let Some(secs) = env_secs_opt(ENV_IDLE_TTL_SECS) {
        return secs.map(Duration::from_secs);
    }
    using_redis.then(|| Duration::from_secs(DEFAULT_IDLE_TTL_SECS))
}

fn max_lifetime_from_env(using_redis: bool) -> Option<Duration> {
    if let Some(secs) = env_secs_opt(ENV_MAX_LIFETIME_SECS) {
        return secs.map(Duration::from_secs);
    }
    using_redis.then(|| Duration::from_secs(DEFAULT_MAX_LIFETIME_SECS))
}

fn purge_interval_from_env(using_redis: bool) -> Option<Duration> {
    if let Some(secs) = env_secs_opt(ENV_PURGE_INTERVAL_SECS) {
        return secs.map(Duration::from_secs);
    }
    using_redis.then(|| Duration::from_secs(DEFAULT_PURGE_INTERVAL_SECS))
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(default)
}

fn env_secs_opt(name: &str) -> Option<Option<u64>> {
    std::env::var(name).ok().map(|raw| {
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed == "0" {
            None
        } else {
            Some(trimmed.parse().unwrap_or(0))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use unigateway_session::FingerprintPolicy;

    #[test]
    fn redis_defaults_to_required_fingerprint() {
        assert_eq!(
            fingerprint_policy_from_env(true),
            FingerprintPolicy::Required
        );
    }

    #[test]
    fn memory_defaults_to_optional_fingerprint() {
        assert_eq!(
            fingerprint_policy_from_env(false),
            FingerprintPolicy::Optional
        );
    }
}
