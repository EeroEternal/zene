//! Filesystem adapter for compaction segment persistence.

use std::path::PathBuf;

use anyhow::{Context, Result};
use zene_session::session_record_dir;

/// Payload for a compaction segment write (no IO).
#[derive(Debug, Clone)]
pub struct CompactionSegmentWrite {
    pub session_id: String,
    pub body: String,
}

/// Runtime adapter for persisting compaction segments.
pub trait CompactionSegmentStore: Send + Sync {
    fn write_segment(&self, write: &CompactionSegmentWrite) -> Result<PathBuf>;
}

/// Default store: `~/.zene/sessions/<id>/compaction_segments/`.
pub struct FsCompactionSegmentStore;

impl FsCompactionSegmentStore {
    pub fn new() -> Self {
        Self
    }
}

impl Default for FsCompactionSegmentStore {
    fn default() -> Self {
        Self::new()
    }
}

impl CompactionSegmentStore for FsCompactionSegmentStore {
    fn write_segment(&self, write: &CompactionSegmentWrite) -> Result<PathBuf> {
        let dir = session_record_dir(&write.session_id).join("compaction_segments");
        std::fs::create_dir_all(&dir).context("create compaction_segments dir")?;
        let ts = chrono::Utc::now().format("%Y%m%dT%H%M%SZ");
        let path = dir.join(format!("{ts}.md"));
        std::fs::write(&path, &write.body).context("write compaction segment")?;
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn fs_store_writes_under_session_dir() {
        let dir = tempdir().unwrap();
        std::env::set_var("ZENE_HOME", dir.path());
        let write = CompactionSegmentWrite {
            session_id: "sess-1".into(),
            body: "# segment\n".into(),
        };
        let path = FsCompactionSegmentStore::new()
            .write_segment(&write)
            .unwrap();
        std::env::remove_var("ZENE_HOME");
        assert!(path.to_string_lossy().contains("compaction_segments"));
        assert!(std::fs::read_to_string(path).unwrap().contains("# segment"));
    }
}
