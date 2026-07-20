use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::http::{HeaderMap, StatusCode};
use serde::Serialize;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct AuthState {
    pub token: String,
    pub bind_host: String,
    pub port: u16,
    idempotency: Arc<Mutex<HashMap<String, IdempotentEntry>>>,
}

#[derive(Debug, Clone)]
struct IdempotentEntry {
    status: StatusCode,
    body: serde_json::Value,
    stored_at: Instant,
}

impl AuthState {
    pub fn new(token: String, bind_host: String, port: u16) -> Self {
        Self {
            token,
            bind_host,
            port,
            idempotency: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn generate_token() -> String {
        format!("zg_{}", Uuid::new_v4().simple())
    }

    pub fn extract_token(headers: &HeaderMap, query_token: Option<&str>) -> Option<String> {
        if let Some(value) = headers.get("x-zene-token").and_then(|v| v.to_str().ok()) {
            return Some(value.to_string());
        }
        if let Some(value) = headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
        {
            let value = value.trim();
            if let Some(rest) = value.strip_prefix("Bearer ") {
                return Some(rest.trim().to_string());
            }
        }
        query_token.map(str::to_string)
    }

    pub fn authorize(&self, headers: &HeaderMap, query_token: Option<&str>) -> Result<(), StatusCode> {
        let Some(provided) = Self::extract_token(headers, query_token) else {
            return Err(StatusCode::UNAUTHORIZED);
        };
        if provided != self.token {
            return Err(StatusCode::UNAUTHORIZED);
        }
        self.check_origin(headers)
    }

    pub fn check_origin(&self, headers: &HeaderMap) -> Result<(), StatusCode> {
        let Some(origin) = headers.get(axum::http::header::ORIGIN) else {
            return Ok(());
        };
        let Ok(origin) = origin.to_str() else {
            return Err(StatusCode::FORBIDDEN);
        };
        if self.allowed_origins().iter().any(|allowed| allowed == origin) {
            Ok(())
        } else {
            Err(StatusCode::FORBIDDEN)
        }
    }

    pub fn allowed_origins(&self) -> Vec<String> {
        let mut origins = vec![
            format!("http://{}:{}", self.bind_host, self.port),
            format!("http://127.0.0.1:{}", self.port),
            format!("http://localhost:{}", self.port),
        ];
        origins.sort();
        origins.dedup();
        origins
    }

    pub async fn recall_idempotent(
        &self,
        request_id: &str,
    ) -> Option<(StatusCode, serde_json::Value)> {
        self.prune_idempotent().await;
        let map = self.idempotency.lock().await;
        map.get(request_id)
            .map(|entry| (entry.status, entry.body.clone()))
    }

    pub async fn store_idempotent(
        &self,
        request_id: String,
        status: StatusCode,
        body: serde_json::Value,
    ) {
        let mut map = self.idempotency.lock().await;
        map.insert(
            request_id,
            IdempotentEntry {
                status,
                body,
                stored_at: Instant::now(),
            },
        );
    }

    async fn prune_idempotent(&self) {
        let mut map = self.idempotency.lock().await;
        let cutoff = Instant::now() - Duration::from_secs(10 * 60);
        map.retain(|_, entry| entry.stored_at >= cutoff);
        while map.len() > 2_000 {
            if let Some(oldest) = map
                .iter()
                .min_by_key(|(_, entry)| entry.stored_at)
                .map(|(key, _)| key.clone())
            {
                map.remove(&oldest);
            } else {
                break;
            }
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    pub error: String,
    pub message: String,
    pub trace_id: String,
    pub retryable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub oldest_cursor: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_cursor: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

impl ErrorBody {
    pub fn new(error: impl Into<String>, message: impl Into<String>, retryable: bool) -> Self {
        Self {
            error: error.into(),
            message: message.into(),
            trace_id: Uuid::new_v4().to_string(),
            retryable,
            oldest_cursor: None,
            latest_cursor: None,
            recovery: None,
        }
    }
}
