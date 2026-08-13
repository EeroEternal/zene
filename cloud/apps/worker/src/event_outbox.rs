use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use tokio::io::AsyncWriteExt;
use uuid::Uuid;
use zene_cloud_domain::{WorkerEventRequest, WorkerFence};

use super::post_event_raw;

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
                tokio::fs::remove_file(&path)
                    .await
                    .with_context(|| format!("remove orphaned event outbox file {}", path.display()))?;
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
                    format!("remove committed event outbox temporary file {}", tmp.display())
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
        self.dir.join(format!("{}.json", event_file_key(source_event_id)))
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
