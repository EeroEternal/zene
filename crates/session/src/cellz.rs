use std::future::Future;
use std::sync::Arc;

use anyhow::{Context, Result};
use reqwest::Client;
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::{
    CompactionEntry, FileSessionStore, SessionEvent, SessionMeta, SessionRecord, SessionStore,
    SessionView, TodoItem,
};

/// Remote HTTP client and session store adapter for the `cellz` daemon.
///
/// `CellzSessionStore` mirrors Zene agent sessions to an isolated Per-Cell SQLite
/// instance managed by a `cellz` daemon, offering sub-millisecond event streaming,
/// single-writer lease arbitration, and automatic blob storage snapshots.
#[derive(Clone)]
pub struct CellzSessionStore {
    client: Client,
    endpoint: String,
    fallback: Arc<FileSessionStore>,
    fallback_on_error: bool,
}

impl std::fmt::Debug for CellzSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CellzSessionStore")
            .field("endpoint", &self.endpoint)
            .field("fallback_on_error", &self.fallback_on_error)
            .finish()
    }
}

impl Default for CellzSessionStore {
    fn default() -> Self {
        Self::from_env()
    }
}

impl CellzSessionStore {
    /// Construct a store pointing to a custom cellz endpoint.
    pub fn new(endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into().trim_end_matches('/').to_string();
        Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default(),
            endpoint,
            fallback: Arc::new(FileSessionStore),
            fallback_on_error: true,
        }
    }

    /// Read `CELLZ_URL` from the environment or default to `http://127.0.0.1:8080`.
    pub fn from_env() -> Self {
        let endpoint = std::env::var("CELLZ_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
        Self::new(endpoint)
    }

    /// Enable or disable silent fallback to local `FileSessionStore` on network or daemon errors.
    pub fn with_fallback(mut self, fallback: bool) -> Self {
        self.fallback_on_error = fallback;
        self
    }

    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }

    /// Helper to execute async HTTP calls from a synchronous context safely across all runtime flavors.
    fn run_async<F, T>(&self, f: F) -> Result<T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("create worker runtime")?;
            rt.block_on(f)
        })
        .join()
        .map_err(|_| anyhow::anyhow!("cellz worker thread panicked"))?
    }

    async fn sync_to_cellz(&self, session: &SessionRecord) -> Result<()> {
        let cell_id = &session.meta.id;

        // 1. Ensure cell is created / active
        let cell_url = format!("{}/api/v1/cells", self.endpoint);
        let create_body = json!({
            "id": cell_id,
            "name": session.meta.title,
            "metadata": {
                "workdir": session.meta.workdir,
                "parent_session_id": session.meta.parent_session_id,
                "parent_sequence": session.meta.parent_sequence,
            }
        });

        let _ = self.client.post(&cell_url).json(&create_body).send().await;

        // 2. Query cell current sequence to avoid sending duplicate events
        let cell_detail_url = format!("{}/api/v1/cells/{}", self.endpoint, cell_id);
        let current_seq = if let Ok(resp) = self.client.get(&cell_detail_url).send().await {
            if resp.status().is_success() {
                if let Ok(data) = resp.json::<Value>().await {
                    data["cell"]["event_sequence"].as_i64().unwrap_or(0)
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        };

        // 3. Collect new events to batch append
        let mut new_event_reqs = Vec::new();
        for event in &session.events {
            let seq = event.sequence() as i64;
            if seq > current_seq {
                let event_type = match event {
                    SessionEvent::MessageAppended { message, .. } => {
                        match message.role {
                            zene_llm::Role::User => "user_message",
                            zene_llm::Role::Assistant => "agent_message",
                            zene_llm::Role::System => "system_message",
                            zene_llm::Role::Tool => "tool_result",
                        }
                    }
                    SessionEvent::TurnStarted { .. } => "turn_start",
                    SessionEvent::TurnEnded { .. } => "turn_end",
                    SessionEvent::ToolCall { .. } => "tool_call",
                    SessionEvent::ToolResult { .. } => "tool_result",
                    SessionEvent::Checkpoint { .. } => "checkpoint",
                    SessionEvent::CompactionApplied { .. } => "compaction",
                    SessionEvent::BranchForked { .. } => "branch_fork",
                    SessionEvent::Rewound { .. } => "rewound",
                    _ => "session_event",
                };

                let turn_id = match event {
                    SessionEvent::TurnStarted { turn_id, .. }
                    | SessionEvent::TurnEnded { turn_id, .. } => Some(turn_id.clone()),
                    SessionEvent::ToolCall { turn_id, .. }
                    | SessionEvent::ToolResult { turn_id, .. } => turn_id.clone(),
                    _ => None,
                };

                new_event_reqs.push(json!({
                    "turn_id": turn_id,
                    "event_type": event_type,
                    "payload": serde_json::to_value(event).unwrap_or(Value::Null)
                }));
            }
        }

        if !new_event_reqs.is_empty() {
            let batch_url = format!("{}/api/v1/cells/{}/events/batch", self.endpoint, cell_id);
            let resp = self
                .client
                .post(&batch_url)
                .json(&json!({ "events": new_event_reqs }))
                .send()
                .await
                .context("send batch events to cellz")?;

            if !resp.status().is_success() {
                let err_text = resp.text().await.unwrap_or_default();
                anyhow::bail!("cellz batch events failed: {}", err_text);
            }
        }

        // 4. Update KV states (todos, compactions, context usage)
        if !session.todos.is_empty() {
            let kv_url = format!("{}/api/v1/cells/{}/kv", self.endpoint, cell_id);
            let _ = self
                .client
                .post(&kv_url)
                .json(&json!({
                    "key": "todos",
                    "value": serde_json::to_value(&session.todos).unwrap_or(Value::Null)
                }))
                .send()
                .await;
        }

        if !session.compactions.is_empty() {
            let kv_url = format!("{}/api/v1/cells/{}/kv", self.endpoint, cell_id);
            let _ = self
                .client
                .post(&kv_url)
                .json(&json!({
                    "key": "compactions",
                    "value": serde_json::to_value(&session.compactions).unwrap_or(Value::Null)
                }))
                .send()
                .await;
        }

        debug!("Synchronized session '{}' to cellz successfully", cell_id);
        Ok(())
    }

    async fn load_from_cellz(&self, id: &str) -> Result<Option<SessionRecord>> {
        let export_url = format!("{}/api/v1/cells/{}/export", self.endpoint, id);
        let resp = match self.client.get(&export_url).send().await {
            Ok(r) => r,
            Err(_) => return Ok(None),
        };

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }

        if !resp.status().is_success() {
            anyhow::bail!("cellz export returned HTTP status: {}", resp.status());
        }

        let body: Value = resp.json().await.context("decode cellz export JSON")?;
        let export = &body["export"];
        if export.is_null() {
            return Ok(None);
        }

        // Reconstruct SessionMeta
        let meta_obj = &export["meta"];
        let meta = SessionMeta {
            id: meta_obj["id"].as_str().unwrap_or(id).to_string(),
            title: meta_obj["name"].as_str().unwrap_or("Untitled").to_string(),
            workdir: meta_obj["metadata"]["workdir"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            created_at: meta_obj["created_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now),
            updated_at: meta_obj["updated_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .unwrap_or_else(chrono::Utc::now),
            parent_session_id: meta_obj["metadata"]["parent_session_id"]
                .as_str()
                .map(String::from),
            parent_sequence: meta_obj["metadata"]["parent_sequence"].as_u64(),
        };

        // Reconstruct events
        let mut events = Vec::new();
        if let Some(events_arr) = export["events"].as_array() {
            for ev in events_arr {
                if let Ok(session_event) = serde_json::from_value::<SessionEvent>(ev["payload"].clone()) {
                    events.push(session_event);
                }
            }
        }

        // Reconstruct todos and compactions from KV
        let todos: Vec<TodoItem> = export["kv"]["todos"]
            .as_array()
            .and_then(|v| serde_json::from_value(Value::Array(v.clone())).ok())
            .unwrap_or_default();

        let compactions: Vec<CompactionEntry> = export["kv"]["compactions"]
            .as_array()
            .and_then(|v| serde_json::from_value(Value::Array(v.clone())).ok())
            .unwrap_or_default();

        let view = SessionView::from_events(&events, &[]);
        let messages = view.messages;

        Ok(Some(SessionRecord {
            meta,
            messages,
            conversation_schema_version: crate::CURRENT_CONVERSATION_SCHEMA_VERSION,
            events,
            event_sequence: export["meta"]["event_sequence"].as_u64().unwrap_or(0),
            compactions,
            todos,
            context_window_usage: None,
            context_tokens_used: None,
        }))
    }
}

impl SessionStore for CellzSessionStore {
    fn save(&self, session: &SessionRecord) -> Result<()> {
        let cellz_res = self.run_async({
            let store = self.clone();
            let session = session.clone();
            async move { store.sync_to_cellz(&session).await }
        });

        match cellz_res {
            Ok(()) => {
                // Also write to local file for dual-write compatibility
                let _ = self.fallback.save(session);
                Ok(())
            }
            Err(e) => {
                if self.fallback_on_error {
                    warn!("Failed to save session to cellz (falling back to disk): {}", e);
                    self.fallback.save(session)
                } else {
                    Err(e)
                }
            }
        }
    }

    fn load(&self, id: &str) -> Result<Option<SessionRecord>> {
        let cellz_res = self.run_async({
            let store = self.clone();
            let id = id.to_string();
            async move { store.load_from_cellz(&id).await }
        });

        match cellz_res {
            Ok(Some(record)) => Ok(Some(record)),
            Ok(None) | Err(_) => {
                // Fallback to local disk file if cellz didn't find or errored
                self.fallback.load(id)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TodoStatus;
    use tempfile::tempdir;

    #[test]
    fn test_cellz_store_fallback_when_unreachable() {
        let _guard = crate::ZENE_HOME_TEST_LOCK.lock().unwrap();
        let dir = tempdir().unwrap();
        let prev = std::env::var("ZENE_HOME").ok();
        std::env::set_var("ZENE_HOME", dir.path());

        let store = CellzSessionStore::new("http://127.0.0.1:1").with_fallback(true);

        let mut session = SessionRecord::new(std::path::Path::new("/tmp/test-workdir"));
        session.meta.title = "Offline Fallback Test".into();
        session.push_message(zene_llm::Message::user("Hello from offline test"));
        session.todos.push(TodoItem {
            id: "todo-1".into(),
            content: "Check fallback".into(),
            status: TodoStatus::Pending,
        });

        // Saving to unreachable endpoint must gracefully succeed by falling back to FileSessionStore
        let save_res = store.save(&session);
        assert!(save_res.is_ok(), "Fallback save must succeed");

        // Loading must also fallback to local file
        let loaded = store.load(&session.meta.id).unwrap();
        assert!(loaded.is_some());
        let loaded = loaded.unwrap();
        assert_eq!(loaded.meta.id, session.meta.id);
        assert_eq!(loaded.meta.title, "Offline Fallback Test");
        assert_eq!(loaded.todos.len(), 1);

        match prev {
            Some(v) => std::env::set_var("ZENE_HOME", v),
            None => std::env::remove_var("ZENE_HOME"),
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_cellz_store_sync_and_load_roundtrip() {
        use axum::extract::Path;
        use axum::routing::{get, post};
        use axum::{Json, Router};
        use std::sync::atomic::{AtomicBool, Ordering};

        let received_create = Arc::new(AtomicBool::new(false));
        let received_batch = Arc::new(AtomicBool::new(false));

        let c_flag = Arc::clone(&received_create);
        let b_flag = Arc::clone(&received_batch);

        let app = Router::new()
            .route(
                "/api/v1/cells",
                post(move || {
                    c_flag.store(true, Ordering::SeqCst);
                    async { Json(json!({ "status": "ok" })) }
                }),
            )
            .route(
                "/api/v1/cells/{id}",
                get(|| async {
                    Json(json!({
                        "cell": {
                            "event_sequence": 0
                        }
                    }))
                }),
            )
            .route(
                "/api/v1/cells/{id}/events/batch",
                post(move || {
                    b_flag.store(true, Ordering::SeqCst);
                    async { Json(json!({ "status": "ok" })) }
                }),
            )
            .route("/api/v1/cells/{id}/kv", post(|| async { Json(json!({ "status": "ok" })) }))
            .route(
                "/api/v1/cells/{id}/export",
                get(|Path(id): Path<String>| async move {
                    Json(json!({
                        "export": {
                            "meta": {
                                "id": id,
                                "name": "Synced Agent",
                                "event_sequence": 2,
                                "created_at": "2026-09-03T18:00:00Z",
                                "updated_at": "2026-09-03T18:05:00Z",
                                "metadata": { "workdir": "/tmp/work" }
                            },
                            "events": [
                                {
                                    "sequence": 1,
                                    "id": "ev-1",
                                    "event_type": "user_message",
                                    "payload": {
                                        "type": "message_appended",
                                        "sequence": 1,
                                        "id": "ev-1",
                                        "created_at": "2026-09-03T18:00:00Z",
                                        "message": { "role": "user", "content": "Roundtrip test" }
                                    }
                                }
                            ],
                            "messages": [],
                            "kv": {
                                "todos": [
                                    { "id": "t1", "content": "Live in cellz", "status": "completed" }
                                ]
                            },
                            "checkpoints": []
                        }
                    }))
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let endpoint = format!("http://127.0.0.1:{}", port);

        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let store = CellzSessionStore::new(endpoint).with_fallback(false);

        let mut session = SessionRecord::new(std::path::Path::new("/tmp/work"));
        session.meta.title = "Synced Agent".into();
        session.push_message(zene_llm::Message::user("Roundtrip test"));
        session.todos.push(TodoItem {
            id: "t1".into(),
            content: "Live in cellz".into(),
            status: TodoStatus::Completed,
        });

        // 1. Save session to mock cellz
        let save_res = store.save(&session);
        assert!(save_res.is_ok(), "Save to cellz must succeed: {:?}", save_res);
        assert!(received_create.load(Ordering::SeqCst), "Should call /cells create");
        assert!(received_batch.load(Ordering::SeqCst), "Should call /events/batch");

        // 2. Load session from mock cellz
        let loaded = store.load(&session.meta.id).unwrap().expect("Must find exported session");
        assert_eq!(loaded.meta.id, session.meta.id);
        assert_eq!(loaded.meta.title, "Synced Agent");
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.todos.len(), 1);
        assert_eq!(loaded.todos[0].content, "Live in cellz");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(loaded.messages[0].content, Some("Roundtrip test".to_string()));
    }
}
