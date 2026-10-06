//! Remote session store over plain HTTP.
//!
//! Contract (see `docs/session-backends.md`): `GET /sessions/{id}` returns the
//! session JSON or 404; `PUT /sessions/{id}` stores the session JSON body and
//! returns 2xx. The reference worker is a Durable Object with SQLite storage;
//! the same bundle runs on Cloudflare and on self-hosted `celld` cells.

use std::future::Future;

use anyhow::{Context, Result};
use reqwest::Client;

use crate::{parse_session_raw, SessionRecord, SessionStore};

#[derive(Clone)]
pub struct HttpSessionStore {
    client: Client,
    endpoint: String,
}

impl std::fmt::Debug for HttpSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpSessionStore")
            .field("endpoint", &self.endpoint)
            .finish()
    }
}

impl HttpSessionStore {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            client: Client::builder().build().expect("reqwest client"),
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
        }
    }

    fn url(&self, id: &str) -> String {
        format!("{}/sessions/{}", self.endpoint, id)
    }

    async fn save_remote(&self, session: &SessionRecord) -> Result<()> {
        let raw = serde_json::to_string(session).context("serialize session")?;
        let response = self
            .client
            .put(self.url(&session.meta.id))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(raw)
            .send()
            .await
            .context("PUT session")?;
        response.error_for_status().context("PUT session status")?;
        Ok(())
    }

    async fn load_remote(&self, id: &str) -> Result<Option<SessionRecord>> {
        let response = self
            .client
            .get(self.url(id))
            .send()
            .await
            .context("GET session")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        let response = response.error_for_status().context("GET session status")?;
        let raw = response.text().await.context("read session body")?;
        parse_session_raw(&raw, Some(id))
            .map(Some)
            .context("parse remote session")
    }
}

impl SessionStore for HttpSessionStore {
    fn save(&self, session: &SessionRecord) -> Result<()> {
        let store = self.clone();
        let session = session.clone();
        block_on_compat(async move { store.save_remote(&session).await })
    }

    fn load(&self, id: &str) -> Result<Option<SessionRecord>> {
        let store = self.clone();
        let id = id.to_string();
        block_on_compat(async move { store.load_remote(&id).await })
    }
}

/// Run a store future from the synchronous [`SessionStore`] API.
fn block_on_compat<F, T>(future: F) -> Result<T>
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
                .unwrap_or_else(|_| Err(anyhow::anyhow!("http session store thread panicked")))
            })
        }
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("build tokio runtime for http session store")?;
        rt.block_on(future)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use std::sync::Arc;

    use axum::extract::Path as AxumPath;
    use axum::http::StatusCode;
    use axum::{Json, Router};
    use tokio::sync::Mutex;

    #[tokio::test(flavor = "multi_thread")]
    async fn save_and_load_roundtrip_and_missing() {
        let stored: Arc<Mutex<Option<String>>> = Arc::default();
        let put_store = stored.clone();
        let get_store = stored.clone();
        let app = Router::new().route(
            "/sessions/{id}",
            axum::routing::put(move |AxumPath(_id): AxumPath<String>, body: String| {
                let store = put_store.clone();
                async move {
                    *store.lock().await = Some(body);
                    (
                        StatusCode::NO_CONTENT,
                        Json(serde_json::json!({"ok": true})),
                    )
                }
            })
            .get(move |AxumPath(_id): AxumPath<String>| {
                let store = get_store.clone();
                async move {
                    match store.lock().await.clone() {
                        Some(raw) => (StatusCode::OK, raw),
                        None => (StatusCode::NOT_FOUND, String::new()),
                    }
                }
            }),
        );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });

        let store = HttpSessionStore::new(format!("http://{addr}"));
        assert!(store.load("missing").unwrap().is_none());

        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(zene_llm::Message::user("hi"));
        store.save(&session).unwrap();
        let loaded = store.load(&session.meta.id).unwrap().expect("saved");
        assert_eq!(loaded.messages, session.messages);
    }
}
