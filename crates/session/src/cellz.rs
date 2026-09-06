use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use cellz::cell::CellManager;
use cellz::model::{
    AppendEventRequest, BatchAppendRequest, CellExport, CreateCellRequest, SetKVRequest,
};
use cellz::storage::LocalBlobStore;
use reqwest::Client;
use serde_json::{json, Value};
use tracing::{debug, warn};

use crate::{
    CompactionEntry, FileSessionStore, SessionEvent, SessionMeta, SessionRecord, SessionStore,
    SessionView, TodoItem,
};

/// Internal driver for Cellz execution: either remote over HTTP/SSE or embedded in-process.
#[derive(Clone)]
pub enum CellzDriver {
    /// Remote HTTP REST client connecting to an external `cellz` daemon or cloud deployment.
    Http { client: Client, endpoint: String },
    /// Embedded in-process SQLite-per-session engine powered by `cellz::cell::CellManager`.
    Embedded { manager: Arc<CellManager> },
}

/// Session store adapter powered by `cellz` (crates.io).
///
/// `CellzSessionStore` mirrors or directly hosts Zene agent sessions in isolated
/// SQLite cells with event sourcing, single-writer lease arbitration, and automatic snapshots.
///
/// It operates in two modes:
/// 1. **Embedded Mode** (`CellzSessionStore::embedded`): Runs the SQLite cell engine directly
///    in-process with 0 network latency, no external daemon required.
/// 2. **Remote Mode** (`CellzSessionStore::new` or `from_env`): Connects to an external
///    `cellz` daemon (e.g. on localhost or Cloud VM) over HTTP/SSE.
#[derive(Clone)]
pub struct CellzSessionStore {
    driver: CellzDriver,
    fallback: Arc<FileSessionStore>,
    fallback_on_error: bool,
}

impl std::fmt::Debug for CellzSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.driver {
            CellzDriver::Http { endpoint, .. } => f
                .debug_struct("CellzSessionStore::Http")
                .field("endpoint", endpoint)
                .field("fallback_on_error", &self.fallback_on_error)
                .finish(),
            CellzDriver::Embedded { .. } => f
                .debug_struct("CellzSessionStore::Embedded")
                .field("fallback_on_error", &self.fallback_on_error)
                .finish(),
        }
    }
}

impl Default for CellzSessionStore {
    fn default() -> Self {
        Self::from_env()
    }
}

impl CellzSessionStore {
    /// Construct a store pointing to a remote cellz HTTP endpoint.
    pub fn new(endpoint: impl Into<String>) -> Self {
        let endpoint = endpoint.into().trim_end_matches('/').to_string();
        Self {
            driver: CellzDriver::Http {
                client: Client::builder()
                    .timeout(std::time::Duration::from_secs(5))
                    .build()
                    .unwrap_or_default(),
                endpoint,
            },
            fallback: Arc::new(FileSessionStore),
            fallback_on_error: true,
        }
    }

    /// Read `CELLZ_URL` from the environment, defaulting to `http://127.0.0.1:8080`.
    pub fn from_env() -> Self {
        let endpoint =
            std::env::var("CELLZ_URL").unwrap_or_else(|_| "http://127.0.0.1:8080".to_string());
        Self::new(endpoint)
    }

    /// Construct an embedded in-process store storing cells under `cells_dir`.
    pub fn embedded(cells_dir: impl AsRef<Path>) -> Result<Self> {
        let cells_dir = cells_dir.as_ref();
        let storage_dir = cells_dir.join("storage");
        std::fs::create_dir_all(cells_dir)
            .with_context(|| format!("create cells dir: {}", cells_dir.display()))?;
        std::fs::create_dir_all(&storage_dir)
            .with_context(|| format!("create storage dir: {}", storage_dir.display()))?;

        let storage = Arc::new(LocalBlobStore::new(&storage_dir));
        let manager = Arc::new(CellManager::new(cells_dir, storage, 60));
        Ok(Self {
            driver: CellzDriver::Embedded { manager },
            fallback: Arc::new(FileSessionStore),
            fallback_on_error: true,
        })
    }

    /// Enable or disable silent fallback to local `FileSessionStore` on errors.
    pub fn with_fallback(mut self, fallback: bool) -> Self {
        self.fallback_on_error = fallback;
        self
    }

    /// Return the remote endpoint if configured in HTTP mode.
    pub fn endpoint(&self) -> Option<&str> {
        match &self.driver {
            CellzDriver::Http { endpoint, .. } => Some(endpoint),
            CellzDriver::Embedded { .. } => None,
        }
    }

    fn run_async<F, T>(&self, future: F) -> Result<T>
    where
        F: Future<Output = Result<T>> + Send + 'static,
        T: Send + 'static,
    {
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread {
                tokio::task::block_in_place(|| handle.block_on(future))
            } else {
                std::thread::scope(|s| {
                    s.spawn(|| {
                        let rt = tokio::runtime::Builder::new_current_thread()
                            .enable_all()
                            .build()
                            .context("build fallback current_thread runtime")?;
                        rt.block_on(future)
                    })
                    .join()
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("cellz worker thread panicked")))
                })
            }
        } else {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .context("build tokio runtime for cellz store")?;
            rt.block_on(future)
        }
    }

    async fn save_to_cellz(&self, session: &SessionRecord) -> Result<()> {
        let cell_id = &session.meta.id;

        // 1. Convert session events to Cellz AppendEventRequest models
        let new_event_reqs = self.collect_append_requests(session, 0);

        match &self.driver {
            CellzDriver::Embedded { manager } => {
                let handle = manager
                    .get_or_activate(cell_id)
                    .await
                    .with_context(|| format!("activate embedded cell '{}'", cell_id))?;

                if !new_event_reqs.is_empty() {
                    handle
                        .append_events_batch(new_event_reqs)
                        .await
                        .context("embedded batch append")?;
                }

                handle
                    .set_kv("title", Value::String(session.meta.title.clone()))
                    .await
                    .context("embedded set title kv")?;

                if !session.todos.is_empty() {
                    handle
                        .set_kv("todos", serde_json::to_value(&session.todos)?)
                        .await
                        .context("embedded set todos kv")?;
                }
                if !session.compactions.is_empty() {
                    handle
                        .set_kv("compactions", serde_json::to_value(&session.compactions)?)
                        .await
                        .context("embedded set compactions kv")?;
                }
            }
            CellzDriver::Http { client, endpoint } => {
                // Ensure cell exists on remote daemon
                let create_url = format!("{}/api/v1/cells", endpoint);
                let create_req = CreateCellRequest {
                    id: Some(cell_id.clone()),
                    name: Some(session.meta.title.clone()),
                    metadata: Some(json!({
                        "workdir": session.meta.workdir,
                        "parent_session_id": session.meta.parent_session_id,
                        "parent_sequence": session.meta.parent_sequence,
                    })),
                };
                let _ = client.post(&create_url).json(&create_req).send().await;

                // Inspect current event sequence to only append delta
                let cell_url = format!("{}/api/v1/cells/{}", endpoint, cell_id);
                let current_seq = if let Ok(resp) = client.get(&cell_url).send().await {
                    if let Ok(info) = resp.json::<Value>().await {
                        info["cell"]["event_sequence"].as_i64().unwrap_or(0)
                    } else {
                        0
                    }
                } else {
                    0
                };

                let delta_reqs = self.collect_append_requests(session, current_seq);
                if !delta_reqs.is_empty() {
                    let batch_url = format!("{}/api/v1/cells/{}/events/batch", endpoint, cell_id);
                    let resp = client
                        .post(&batch_url)
                        .json(&BatchAppendRequest { events: delta_reqs })
                        .send()
                        .await
                        .context("send batch events to cellz")?;

                    if !resp.status().is_success() {
                        let err_text = resp.text().await.unwrap_or_default();
                        anyhow::bail!("cellz batch events failed: {}", err_text);
                    }
                }

                if !session.todos.is_empty() {
                    let kv_url = format!("{}/api/v1/cells/{}/kv", endpoint, cell_id);
                    let _ = client
                        .post(&kv_url)
                        .json(&SetKVRequest {
                            key: "todos".into(),
                            value: serde_json::to_value(&session.todos).unwrap_or(Value::Null),
                        })
                        .send()
                        .await;
                }

                if !session.compactions.is_empty() {
                    let kv_url = format!("{}/api/v1/cells/{}/kv", endpoint, cell_id);
                    let _ = client
                        .post(&kv_url)
                        .json(&SetKVRequest {
                            key: "compactions".into(),
                            value: serde_json::to_value(&session.compactions)
                                .unwrap_or(Value::Null),
                        })
                        .send()
                        .await;
                }
            }
        }

        debug!("Synchronized session '{}' to cellz successfully", cell_id);
        Ok(())
    }

    fn collect_append_requests(
        &self,
        session: &SessionRecord,
        since_seq: i64,
    ) -> Vec<AppendEventRequest> {
        let mut reqs = Vec::new();
        for event in &session.events {
            let seq = event.sequence() as i64;
            if seq > since_seq {
                let event_type = match event {
                    SessionEvent::MessageAppended { message, .. } => match message.role {
                        zene_llm::Role::User => "user_message",
                        zene_llm::Role::Assistant => "agent_message",
                        zene_llm::Role::System => "system_message",
                        zene_llm::Role::Tool => "tool_result",
                    },
                    SessionEvent::TurnStarted { .. } => "turn_start",
                    SessionEvent::TurnEnded { .. } => "turn_end",
                    SessionEvent::ToolCall { .. } => "tool_call",
                    SessionEvent::ToolResult { .. } => "tool_result",
                    SessionEvent::Checkpoint { .. } => "checkpoint",
                    SessionEvent::CompactionApplied { .. } => "compaction",
                    SessionEvent::BranchForked { .. } => "branch_fork",
                    SessionEvent::BranchSummary { .. } => "branch_summary",
                    SessionEvent::Label { .. } => "label",
                    SessionEvent::Custom { .. } => "custom",
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

                reqs.push(AppendEventRequest {
                    turn_id,
                    event_type: event_type.to_string(),
                    payload: serde_json::to_value(event).unwrap_or(Value::Null),
                });
            }
        }
        reqs
    }

    async fn load_from_cellz(&self, id: &str) -> Result<Option<SessionRecord>> {
        let export = match &self.driver {
            CellzDriver::Embedded { manager } => {
                let handle = match manager.get_or_activate(id).await {
                    Ok(h) => h,
                    Err(_) => return Ok(None),
                };
                match handle.export().await {
                    Ok(exp) => exp,
                    Err(_) => return Ok(None),
                }
            }
            CellzDriver::Http { client, endpoint } => {
                let export_url = format!("{}/api/v1/cells/{}/export", endpoint, id);
                let resp = match client.get(&export_url).send().await {
                    Ok(r) => r,
                    Err(_) => return Ok(None),
                };

                if resp.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }

                if !resp.status().is_success() {
                    let err = resp.text().await.unwrap_or_default();
                    anyhow::bail!("cellz export error: {}", err);
                }

                let data: Value = resp.json().await.context("decode cellz export json")?;
                let export_val = data.get("export").unwrap_or(&data).clone();
                serde_json::from_value::<CellExport>(export_val)
                    .context("deserialize CellExport")?
            }
        };

        self.convert_cellz_export(export, id).map(Some)
    }

    fn convert_cellz_export(&self, export: CellExport, id: &str) -> Result<SessionRecord> {
        let meta = SessionMeta {
            id: if export.meta.id.is_empty() {
                id.to_string()
            } else {
                export.meta.id
            },
            title: export
                .kv
                .get("title")
                .and_then(|v| v.as_str())
                .or_else(|| export.meta.metadata.get("title").and_then(|v| v.as_str()))
                .unwrap_or(if export.meta.name.is_empty() {
                    "Untitled"
                } else {
                    &export.meta.name
                })
                .to_string(),
            workdir: export.meta.metadata["workdir"]
                .as_str()
                .unwrap_or("")
                .to_string(),
            created_at: export.meta.created_at,
            updated_at: export.meta.updated_at,
            parent_session_id: export.meta.metadata["parent_session_id"]
                .as_str()
                .map(String::from),
            parent_sequence: export.meta.metadata["parent_sequence"].as_u64(),
        };

        let mut events = Vec::new();
        for ev in export.events {
            if let Ok(session_event) = serde_json::from_value::<SessionEvent>(ev.payload) {
                events.push(session_event);
            }
        }

        let todos: Vec<TodoItem> = export
            .kv
            .get("todos")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let compactions: Vec<CompactionEntry> = export
            .kv
            .get("compactions")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        let view = SessionView::from_events(&events, &[]);
        let messages = view.messages;

        Ok(SessionRecord {
            meta,
            messages,
            conversation_schema_version: crate::CURRENT_CONVERSATION_SCHEMA_VERSION,
            events,
            event_sequence: export.meta.event_sequence as u64,
            compactions,
            todos,
            context_window_usage: None,
            context_tokens_used: None,
        })
    }
}

impl SessionStore for CellzSessionStore {
    fn save(&self, session: &SessionRecord) -> Result<()> {
        let cellz_res = self.run_async({
            let store = self.clone();
            let session = session.clone();
            async move { store.save_to_cellz(&session).await }
        });

        match cellz_res {
            Ok(()) => {
                // Mirror write to local filesystem fallback so disk cache stays warm
                let _ = self.fallback.save(session);
                Ok(())
            }
            Err(e) => {
                if self.fallback_on_error {
                    warn!(
                        "Failed to save session to cellz (falling back to disk): {}",
                        e
                    );
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
            Ok(Some(session)) => Ok(Some(session)),
            Ok(None) => {
                // Not found in cellz; check local fallback store
                self.fallback.load(id)
            }
            Err(e) => {
                if self.fallback_on_error {
                    warn!(
                        "Failed to load session from cellz (falling back to disk): {}",
                        e
                    );
                    self.fallback.load(id)
                } else {
                    Err(e)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicBool, Ordering};

    use axum::extract::Path;
    use axum::routing::{get, post};
    use axum::{Json, Router};

    use super::*;
    use crate::TodoStatus;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cellz_store_embedded_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = CellzSessionStore::embedded(tmp.path()).unwrap();

        let mut session = SessionRecord::new(std::path::Path::new("/tmp/work"));
        session.meta.title = "Embedded Agent".into();
        session.push_message(zene_llm::Message::user("Hello from embedded cellz!"));
        session.todos.push(TodoItem {
            id: "t-embed".into(),
            content: "In-process SQLite".into(),
            status: TodoStatus::Pending,
        });

        // 1. Save to embedded SQLite cell
        store.save(&session).expect("save must succeed");

        // 2. Load from embedded SQLite cell
        let loaded = store
            .load(&session.meta.id)
            .unwrap()
            .expect("must load embedded session");
        assert_eq!(loaded.meta.id, session.meta.id);
        assert_eq!(loaded.meta.title, "Embedded Agent");
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.todos.len(), 1);
        assert_eq!(loaded.todos[0].content, "In-process SQLite");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(
            loaded.messages[0].content,
            Some("Hello from embedded cellz!".to_string())
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cellz_store_fallback_when_unreachable() {
        // Point to an unreachable port with fallback enabled
        let store = CellzSessionStore::new("http://127.0.0.1:19999").with_fallback(true);

        let tmp = tempfile::tempdir().unwrap();
        let session = SessionRecord::new(tmp.path());

        // Save should succeed by falling back to FileSessionStore
        let save_res = store.save(&session);
        assert!(
            save_res.is_ok(),
            "Fallback to local disk must succeed on connection failure: {:?}",
            save_res
        );

        // Load should also fallback and retrieve the locally written session
        let loaded = store.load(&session.meta.id);
        assert!(
            loaded.is_ok(),
            "Fallback load must succeed: {:?}",
            loaded.err()
        );
        let loaded = loaded.unwrap();
        assert!(loaded.is_some());
        assert_eq!(loaded.unwrap().meta.id, session.meta.id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_cellz_store_sync_and_load_roundtrip() {
        let received_create = Arc::new(AtomicBool::new(false));
        let received_batch = Arc::new(AtomicBool::new(false));

        let c_flag = received_create.clone();
        let b_flag = received_batch.clone();

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
            .route(
                "/api/v1/cells/{id}/kv",
                post(|| async { Json(json!({ "status": "ok" })) }),
            )
            .route(
                "/api/v1/cells/{id}/export",
                get(|Path(id): Path<String>| async move {
                    Json(json!({
                        "export": {
                            "meta": {
                                "id": id,
                                "name": "Synced Agent",
                                "status": "active",
                                "event_sequence": 2,
                                "created_at": "2026-09-03T18:00:00Z",
                                "updated_at": "2026-09-03T18:05:00Z",
                                "metadata": { "workdir": "/tmp/work" }
                            },
                            "events": [
                                {
                                    "sequence": 1,
                                    "id": "ev-1",
                                    "cell_id": id.clone(),
                                    "turn_id": null,
                                    "event_type": "user_message",
                                    "payload": {
                                        "type": "message_appended",
                                        "sequence": 1,
                                        "id": "ev-1",
                                        "created_at": "2026-09-03T18:00:00Z",
                                        "message": { "role": "user", "content": "Roundtrip test" }
                                    },
                                    "created_at": "2026-09-03T18:00:00Z"
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
        assert!(
            save_res.is_ok(),
            "Save to cellz must succeed: {:?}",
            save_res
        );
        assert!(
            received_create.load(Ordering::SeqCst),
            "Should call /cells create"
        );
        assert!(
            received_batch.load(Ordering::SeqCst),
            "Should call /events/batch"
        );

        // 2. Load session from mock cellz
        let loaded = store
            .load(&session.meta.id)
            .unwrap()
            .expect("Must find exported session");
        assert_eq!(loaded.meta.id, session.meta.id);
        assert_eq!(loaded.meta.title, "Synced Agent");
        assert_eq!(loaded.events.len(), 1);
        assert_eq!(loaded.todos.len(), 1);
        assert_eq!(loaded.todos[0].content, "Live in cellz");
        assert_eq!(loaded.messages.len(), 1);
        assert_eq!(
            loaded.messages[0].content,
            Some("Roundtrip test".to_string())
        );
    }
}
