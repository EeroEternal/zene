use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;
use zene_cloud_domain::{WorkerEventRequest, WorkerFence};

use crate::api::post_event_raw;

const MAX_OUTBOX_EVENTS: usize = 10_000;
const MAX_OUTBOX_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Debug, Clone)]
pub(crate) struct EventOutbox {
    pub(crate) dir: PathBuf,
}

#[allow(dead_code)]
struct OutboxLock(std::fs::File);

impl EventOutbox {
    async fn acquire_lock(&self) -> Result<OutboxLock> {
        let path = self.dir.join(".lock");
        tokio::task::spawn_blocking(move || {
            let file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .open(&path)
                .with_context(|| format!("open event outbox lock {}", path.display()))?;
            #[cfg(unix)]
            {
                use std::os::fd::AsRawFd;
                let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
                if result != 0 {
                    return Err(std::io::Error::last_os_error())
                        .with_context(|| format!("lock event outbox {}", path.display()));
                }
            }
            Ok(OutboxLock(file))
        })
        .await
        .context("join event outbox lock")?
    }

    pub(crate) async fn open(root: &Path, run_id: Uuid) -> Result<Self> {
        let dir = root.join(".event-outbox").join(run_id.to_string());
        tokio::fs::create_dir_all(&dir)
            .await
            .with_context(|| format!("create event outbox {}", dir.display()))?;
        let outbox = Self { dir };
        let _lock = outbox.acquire_lock().await?;
        Self::remove_orphaned_temporary_files(&outbox.dir).await?;
        Ok(outbox)
    }

    async fn remove_orphaned_temporary_files(dir: &Path) -> Result<()> {
        let mut entries = tokio::fs::read_dir(dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with('.') && name.contains(".tmp-") {
                tokio::fs::remove_file(&path).await.with_context(|| {
                    format!("remove orphaned event outbox file {}", path.display())
                })?;
            }
        }
        Ok(())
    }

    pub(crate) async fn enqueue(&self, event: &WorkerEventRequest) -> Result<()> {
        let _lock = self.acquire_lock().await?;
        let path = self.event_path(&event.source_event_id);
        if tokio::fs::try_exists(&path).await? {
            let existing: WorkerEventRequest = serde_json::from_slice(
                &tokio::fs::read(&path)
                    .await
                    .with_context(|| format!("read existing event outbox {}", path.display()))?,
            )
            .with_context(|| format!("decode existing event outbox {}", path.display()))?;
            if existing.source_event_id == event.source_event_id {
                return Ok(());
            }
            bail!(
                "event outbox key collision between source IDs {:?} and {:?}",
                existing.source_event_id,
                event.source_event_id
            );
        }

        let tmp = self.dir.join(format!(
            ".{}.tmp-{}",
            path.file_name().unwrap().to_string_lossy(),
            Uuid::new_v4()
        ));
        let bytes = serde_json::to_vec(event).context("serialize event outbox entry")?;
        let (event_count, byte_count) = self.stats().await?;
        if event_count >= MAX_OUTBOX_EVENTS {
            bail!("event outbox capacity exceeded: {MAX_OUTBOX_EVENTS} events");
        }
        if byte_count.saturating_add(bytes.len() as u64) > MAX_OUTBOX_BYTES {
            bail!("event outbox capacity exceeded: {MAX_OUTBOX_BYTES} bytes");
        }

        let mut file = tokio::fs::File::create(&tmp)
            .await
            .with_context(|| format!("create event outbox {}", tmp.display()))?;
        file.write_all(&bytes)
            .await
            .with_context(|| format!("write event outbox {}", tmp.display()))?;
        file.sync_all()
            .await
            .with_context(|| format!("sync event outbox {}", tmp.display()))?;
        drop(file);

        match tokio::fs::hard_link(&tmp, &path).await {
            Ok(()) => {
                tokio::fs::remove_file(&tmp).await.with_context(|| {
                    format!(
                        "remove committed event outbox temporary file {}",
                        tmp.display()
                    )
                })?;
                sync_outbox_directory(&self.dir).await?;
                Ok(())
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing: WorkerEventRequest = serde_json::from_slice(
                    &tokio::fs::read(&path)
                        .await
                        .with_context(|| format!("read raced event outbox {}", path.display()))?,
                )
                .with_context(|| format!("decode raced event outbox {}", path.display()))?;
                tokio::fs::remove_file(&tmp).await.with_context(|| {
                    format!("remove raced event outbox temporary file {}", tmp.display())
                })?;
                if existing.source_event_id == event.source_event_id {
                    Ok(())
                } else {
                    bail!(
                        "event outbox key collision between source IDs {:?} and {:?}",
                        existing.source_event_id,
                        event.source_event_id
                    );
                }
            }
            Err(err) => Err(err).with_context(|| format!("commit event outbox {}", path.display())),
        }
    }

    pub(crate) async fn stats(&self) -> Result<(usize, u64)> {
        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        let mut count = 0usize;
        let mut bytes = 0u64;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            count = count.saturating_add(1);
            bytes = bytes.saturating_add(entry.metadata().await?.len());
        }
        Ok((count, bytes))
    }

    pub(crate) async fn flush(
        &self,
        client: &reqwest::Client,
        api_url: &str,
        token: &str,
        run_id: Uuid,
        fence: &WorkerFence,
    ) -> Result<()> {
        let mut entries = tokio::fs::read_dir(&self.dir).await?;
        let mut paths = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
                paths.push(path);
            }
        }
        paths.sort();

        for path in paths {
            let bytes = tokio::fs::read(&path)
                .await
                .with_context(|| format!("read event outbox {}", path.display()))?;
            let event: WorkerEventRequest = serde_json::from_slice(&bytes)
                .with_context(|| format!("decode event outbox {}", path.display()))?;
            post_event_raw(client, api_url, token, run_id, event, fence).await?;
            match tokio::fs::remove_file(&path).await {
                Ok(()) => sync_outbox_directory(&self.dir).await?,
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                    // A replacement worker may have acknowledged and removed
                    // the same idempotent event concurrently.
                }
                Err(err) => {
                    return Err(err)
                        .with_context(|| format!("remove event outbox {}", path.display()));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn event_path(&self, source_event_id: &str) -> PathBuf {
        self.dir
            .join(format!("{}.json", event_file_key(source_event_id)))
    }
}

async fn sync_outbox_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let path = path.to_path_buf();
        tokio::task::spawn_blocking(move || -> Result<()> {
            let directory = std::fs::File::open(&path)
                .with_context(|| format!("open event outbox directory {}", path.display()))?;
            directory
                .sync_all()
                .with_context(|| format!("sync event outbox directory {}", path.display()))?;
            Ok(())
        })
        .await
        .context("join event outbox directory sync")??;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn event_file_key(source_event_id: &str) -> String {
    // Keep filenames bounded and deterministic across worker restarts. The
    // fixed FNV-1a pair avoids DefaultHasher's process-specific seed and keeps
    // arbitrary provider event IDs below filesystem filename limits.
    let mut first = 0xcbf29ce484222325u64;
    let mut second = 0x84222325cbf29ce4u64;
    for byte in source_event_id.as_bytes() {
        first ^= u64::from(*byte);
        first = first.wrapping_mul(0x100000001b3);
        second ^= u64::from(*byte).rotate_left(17);
        second = second.wrapping_mul(0x100000001b3);
    }
    format!("{first:016x}{second:016x}")
}

#[cfg(test)]
mod tests {
    use super::{event_file_key, EventOutbox};
    use futures::StreamExt;
    use serde_json::json;
    use std::future::IntoFuture;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use uuid::Uuid;
    use zene_cloud_domain::{RunEventKind, WorkerEventRequest, WorkerFence};

    fn event(source_event_id: &str) -> WorkerEventRequest {
        WorkerEventRequest {
            source_event_id: source_event_id.into(),
            cursor: Some(7),
            event_type: RunEventKind::Acp,
            payload: json!({"ok": true}),
            fence: None,
        }
    }

    fn http_client() -> reqwest::Client {
        reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("build test HTTP client")
    }

    #[test]
    fn event_file_key_is_stable_and_safe() {
        let key = event_file_key("acp/event/1");
        assert_eq!(key.len(), 32);
        assert!(!key.contains('/'));
        assert_eq!(event_file_key("acp/event/1"), key);
        assert_eq!(event_file_key(&"x".repeat(100_000)).len(), 32);
    }

    #[tokio::test]
    async fn event_outbox_survives_reopen_without_tmp_files() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let outbox = EventOutbox::open(&root, run_id).await.unwrap();
        let event = event("event-1");
        outbox.enqueue(&event).await.unwrap();
        outbox.enqueue(&event).await.unwrap();
        assert_eq!(outbox.stats().await.unwrap().0, 1);
        let orphan = outbox.dir.join(".orphan.json.tmp-crashed");
        tokio::fs::write(&orphan, b"partial").await.unwrap();

        let reopened = EventOutbox::open(&root, run_id).await.unwrap();
        let path = reopened.event_path("event-1");
        let stored: WorkerEventRequest =
            serde_json::from_slice(&tokio::fs::read(&path).await.unwrap()).unwrap();
        assert_eq!(stored.source_event_id, "event-1");
        assert_eq!(stored.cursor, Some(7));
        assert!(!orphan.exists());
        let mut entries = tokio::fs::read_dir(&reopened.dir).await.unwrap();
        let mut event_files = Vec::new();
        while let Some(entry) = entries.next_entry().await.unwrap() {
            if entry.path().extension().and_then(|ext| ext.to_str()) == Some("json") {
                event_files.push(entry.path());
            }
        }
        assert_eq!(event_files.len(), 1);

        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_concurrent_same_event_is_idempotent() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let outbox = EventOutbox::open(&root, Uuid::new_v4()).await.unwrap();
        let event = event("concurrent-event");
        let (left, right) = tokio::join!(outbox.enqueue(&event), outbox.enqueue(&event));
        left.unwrap();
        right.unwrap();
        assert_eq!(
            outbox.stats().await.unwrap(),
            (
                1,
                outbox
                    .event_path("concurrent-event")
                    .metadata()
                    .unwrap()
                    .len()
            )
        );
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_rejects_source_id_collision() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let outbox = EventOutbox::open(&root, Uuid::new_v4()).await.unwrap();
        let first = event("first");
        let second = event("second");
        tokio::fs::write(
            outbox.event_path("first"),
            serde_json::to_vec(&second).unwrap(),
        )
        .await
        .unwrap();
        let error = outbox
            .enqueue(&first)
            .await
            .expect_err("collision must fail");
        assert!(error.to_string().contains("collision"));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_retries_transient_http_failure_before_acknowledging() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let outbox = EventOutbox::open(&root, run_id).await.unwrap();
        outbox.enqueue(&event("retry-event")).await.unwrap();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for response in [
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".as_slice(),
        ] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).await.unwrap();
            stream.write_all(response).await.unwrap();
        }
        });
        let fence = WorkerFence {
            attempt_id: Uuid::new_v4(),
            generation: 1,
            worker_id: "worker-retry".into(),
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            outbox.flush(
                &http_client(),
                &format!("http://{address}"),
                "worker-token",
                run_id,
                &fence,
            ),
        )
        .await
        .expect("retrying flush should complete")
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .expect("retry server should complete")
            .unwrap();
        assert_eq!(outbox.stats().await.unwrap(), (0, 0));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_retains_event_after_non_retryable_http_failure() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let outbox = EventOutbox::open(&root, run_id).await.unwrap();
        let queued = event("rejected-event");
        outbox.enqueue(&queued).await.unwrap();

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buffer = [0_u8; 4096];
            let _ = stream.read(&mut buffer).await.unwrap();
            stream
                .write_all(
                    b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
        });
        let fence = WorkerFence {
            attempt_id: Uuid::new_v4(),
            generation: 1,
            worker_id: "worker-reject".into(),
        };
        let error = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            outbox.flush(
                &http_client(),
                &format!("http://{address}"),
                "worker-token",
                run_id,
                &fence,
            ),
        )
        .await
        .expect("non-retryable flush should complete")
        .expect_err("HTTP 400 must be surfaced");
        assert!(error.to_string().contains("400"));
        tokio::time::timeout(std::time::Duration::from_secs(5), server)
            .await
            .expect("reject server should complete")
            .unwrap();
        assert_eq!(outbox.stats().await.unwrap().0, 1);
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_reconnects_to_real_api_and_sse_replays_after_replacement() {
        use zene_cloud_api::{router, AppState};
        use zene_cloud_db::Db;
        use zene_cloud_domain::{
            CreateRepositoryRequest, CreateRunRequest, PermissionMode, RegisterRequest, RunEvent,
            RunStatus, UpdateLlmSettingsRequest, WorkerClaimRequest,
        };
        use zene_cloud_github::{GithubClient, GithubConfig};

        let db = Db::connect("sqlite::memory:").await.unwrap();
        db.migrate().await.unwrap();
        let worker_token = "real-api-worker-token";
        db.ensure_dev_worker_token(worker_token).await.unwrap();
        let root = std::env::temp_dir().join(format!("zene-worker-api-{}", Uuid::new_v4()));
        let workspace_root = root.join("workspaces");
        tokio::fs::create_dir_all(&workspace_root).await.unwrap();
        let state = AppState::new(
            db.clone(),
            worker_token.into(),
            GithubClient::new(GithubConfig::live_default()),
            workspace_root.clone(),
            "http://127.0.0.1".into(),
        );
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let api_task = tokio::spawn(axum::serve(listener, router(state.clone())).into_future());
        let api_url = format!("http://{address}");
        let client = http_client();

        let auth: zene_cloud_domain::AuthResponse = client
            .post(format!("{api_url}/api/v1/auth/register"))
            .json(&RegisterRequest {
                email: "worker-reconnect@example.com".into(),
                password: "password123".into(),
                display_name: "Worker Reconnect".into(),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let repo: zene_cloud_domain::Repository = client
            .post(format!("{api_url}/api/v1/repositories"))
            .bearer_auth(&auth.token)
            .json(&CreateRepositoryRequest {
                owner: "worker".into(),
                name: "reconnect".into(),
                default_branch: "main".into(),
                clone_url: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        db.upsert_user_llm_settings(
            auth.user.id,
            UpdateLlmSettingsRequest {
                provider_id: "test".into(),
                base_url: "https://llm.invalid".into(),
                default_model: "default".into(),
                models: vec!["default".into()],
                api_key: Some("test-key".into()),
            },
        )
        .await
        .unwrap();
        let run: zene_cloud_domain::Run = client
            .post(format!("{api_url}/api/v1/runs"))
            .bearer_auth(&auth.token)
            .json(&CreateRunRequest {
                repository_id: repo.id,
                prompt: "worker reconnect".into(),
                base_ref: Some("main".into()),
                model: "default".into(),
                permission_mode: PermissionMode::Default,
                max_turns: 10,
                mode_id: None,
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();

        let claim: zene_cloud_domain::ClaimedRun = client
            .post(format!("{api_url}/internal/v1/runs/claim"))
            .bearer_auth(worker_token)
            .json(&WorkerClaimRequest {
                worker_id: "worker-1".into(),
                workspace_root: workspace_root.to_string_lossy().into(),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(claim.run.id, run.id);
        let first_root = root.join("first-worker");
        let first_outbox = EventOutbox::open(&first_root, run.id).await.unwrap();
        let first_event = WorkerEventRequest {
            source_event_id: "real-provider-event-1".into(),
            cursor: Some(21),
            event_type: RunEventKind::Runtime,
            payload: json!({"marker": "first"}),
            fence: None,
        };
        first_outbox.enqueue(&first_event).await.unwrap();
        drop(first_outbox);

        db.update_run_status(run.id, RunStatus::Failed, None, Some("worker_lost".into()))
            .await
            .unwrap();
        db.update_run_status(run.id, RunStatus::Queued, None, None)
            .await
            .unwrap();
        let replacement: zene_cloud_domain::ClaimedRun = client
            .post(format!("{api_url}/internal/v1/runs/claim"))
            .bearer_auth(worker_token)
            .json(&WorkerClaimRequest {
                worker_id: "worker-2".into(),
                workspace_root: workspace_root.to_string_lossy().into(),
            })
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        let second_fence = WorkerFence {
            attempt_id: replacement.attempt_id,
            generation: replacement.generation,
            worker_id: "worker-2".into(),
        };
        let replacement_outbox = EventOutbox::open(&first_root, run.id).await.unwrap();
        replacement_outbox
            .flush(&client, &api_url, worker_token, run.id, &second_fence)
            .await
            .unwrap();
        assert_eq!(replacement_outbox.stats().await.unwrap(), (0, 0));

        let first: RunEvent = db
            .events_after(run.id, 0)
            .await
            .unwrap()
            .into_iter()
            .find(|event| event.event_type == "runtime")
            .unwrap();
        let second_event = WorkerEventRequest {
            source_event_id: "real-provider-event-2".into(),
            cursor: Some(22),
            event_type: RunEventKind::Runtime,
            payload: json!({"marker": "second"}),
            fence: None,
        };
        replacement_outbox.enqueue(&second_event).await.unwrap();
        replacement_outbox
            .flush(&client, &api_url, worker_token, run.id, &second_fence)
            .await
            .unwrap();

        let response = client
            .get(format!("{api_url}/api/v1/runs/{}/events/stream", run.id))
            .bearer_auth(&auth.token)
            .header("Last-Event-ID", first.seq.to_string())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
        let mut stream = response.bytes_stream();
        let mut sse = String::new();
        while !sse.contains("second") {
            let chunk = tokio::time::timeout(Duration::from_secs(3), stream.next())
                .await
                .expect("SSE replay timeout")
                .expect("SSE closed")
                .unwrap();
            sse.push_str(&String::from_utf8_lossy(&chunk));
        }
        assert!(sse.contains("second"));
        assert!(!sse.contains("first"));

        api_task.abort();
        let _ = api_task.await;
        tokio::fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn event_outbox_reopen_flushes_pending_event_and_removes_acknowledged_file() {
        let root = std::env::temp_dir().join(format!("zene-worker-outbox-{}", Uuid::new_v4()));
        let run_id = Uuid::new_v4();
        let first_worker = EventOutbox::open(&root, run_id).await.unwrap();
        first_worker.enqueue(&event("restart-event")).await.unwrap();
        drop(first_worker);

        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 4096];
            let body_start = loop {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending request");
                request.extend_from_slice(&buffer[..read]);
                if let Some(position) = request.windows(4).position(|window| window == b"\r\n\r\n")
                {
                    break position + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..body_start]);
            let content_length = headers
                .lines()
                .find_map(|line| {
                    line.strip_prefix("Content-Length:")
                        .or_else(|| line.strip_prefix("content-length:"))
                })
                .and_then(|value| value.trim().parse::<usize>().ok())
                .expect("event POST should include content length");
            while request.len() < body_start + content_length {
                let read = stream.read(&mut buffer).await.unwrap();
                assert!(read > 0, "client closed before sending request body");
                request.extend_from_slice(&buffer[..read]);
            }
            let payload: WorkerEventRequest =
                serde_json::from_slice(&request[body_start..body_start + content_length]).unwrap();
            assert_eq!(payload.source_event_id, "restart-event");
            assert_eq!(payload.cursor, Some(7));
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .await
                .unwrap();
        });

        let reopened = EventOutbox::open(&root, run_id).await.unwrap();
        let fence = WorkerFence {
            attempt_id: Uuid::new_v4(),
            generation: 2,
            worker_id: "replacement-worker".into(),
        };
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            reopened.flush(
                &http_client(),
                &format!("http://{address}"),
                "worker-token",
                run_id,
                &fence,
            ),
        )
        .await
        .expect("outbox flush should complete")
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(10), server)
            .await
            .expect("mock server should complete")
            .unwrap();
        assert_eq!(reopened.stats().await.unwrap(), (0, 0));
        tokio::fs::remove_dir_all(root).await.unwrap();
    }
}
