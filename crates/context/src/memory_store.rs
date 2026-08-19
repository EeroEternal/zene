//! Filesystem adapter for session memory persistence.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Runtime adapter for durable memory files under `.zene/memory/`.
pub trait MemoryStore: Send + Sync {
    fn load_recent(&self) -> Option<String>;
    fn is_duplicate_flush(&self, content: &str) -> bool;
    fn append_daily_log(&self, content: &str) -> Result<PathBuf>;
}

const MAX_INJECT_CHARS: usize = 3_000;

fn memory_root(workdir: &Path) -> PathBuf {
    workdir.join(".zene").join("memory")
}

fn daily_log_path(workdir: &Path) -> PathBuf {
    let day = chrono::Utc::now().format("%Y-%m-%d");
    memory_root(workdir).join("daily").join(format!("{day}.md"))
}

fn last_flush_hash_path(workdir: &Path) -> PathBuf {
    memory_root(workdir).join(".last_flush_hash")
}

fn content_fingerprint(content: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    content.trim().hash(&mut h);
    h.finish()
}

/// Default store: `{workdir}/.zene/memory/`.
pub struct FsMemoryStore {
    workdir: PathBuf,
}

impl FsMemoryStore {
    pub fn new(workdir: impl Into<PathBuf>) -> Self {
        Self {
            workdir: workdir.into(),
        }
    }

    pub fn workdir(&self) -> &Path {
        &self.workdir
    }
}

impl MemoryStore for FsMemoryStore {
    fn load_recent(&self) -> Option<String> {
        let root = memory_root(&self.workdir);
        let mut chunks = Vec::new();

        let memory_md = root.join("MEMORY.md");
        if let Ok(text) = std::fs::read_to_string(&memory_md) {
            if !text.trim().is_empty() {
                chunks.push(text);
            }
        }

        // Load active agent notes (.zene/notes/active or docs/notes/active)
        for notes_parent in &[
            self.workdir.join(".zene").join("notes"),
            self.workdir.join("docs").join("notes"),
        ] {
            let active_dir = notes_parent.join("active");
            if let Ok(entries) = std::fs::read_dir(&active_dir) {
                let mut note_files: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
                    .collect();
                note_files.sort();
                for path in note_files {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            chunks.push(format!(
                                "### Invariant Note ({})\n\n{trimmed}",
                                path.file_name().unwrap_or_default().to_string_lossy()
                            ));
                        }
                    }
                }
            }
        }

        let daily_dir = root.join("daily");
        if let Ok(entries) = std::fs::read_dir(&daily_dir) {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
                .collect();
            files.sort();
            for path in files.iter().rev().take(3) {
                if let Ok(text) = std::fs::read_to_string(path) {
                    if !text.trim().is_empty() {
                        chunks.push(text);
                    }
                }
            }
        }

        if chunks.is_empty() {
            return None;
        }
        let mut combined = chunks.join("\n\n");
        if combined.chars().count() > MAX_INJECT_CHARS {
            let owned: String = combined
                .chars()
                .rev()
                .take(MAX_INJECT_CHARS)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            combined = format!("…\n{owned}");
        }
        Some(combined)
    }

    fn is_duplicate_flush(&self, content: &str) -> bool {
        let path = last_flush_hash_path(&self.workdir);
        let Ok(prev) = std::fs::read_to_string(path) else {
            return false;
        };
        prev.trim() == content_fingerprint(content).to_string()
    }

    fn append_daily_log(&self, content: &str) -> Result<PathBuf> {
        let path = daily_log_path(&self.workdir);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).context("create memory daily dir")?;
        }
        let mut block = String::new();
        if path.exists() {
            block.push_str("\n\n---\n\n");
        }
        block.push_str(&format!(
            "## Flush {}\n\n{content}\n",
            chrono::Utc::now().format("%H:%M:%SZ")
        ));
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .context("open daily memory log")?;
        file.write_all(block.as_bytes())
            .context("write daily memory log")?;
        let _ = std::fs::write(
            last_flush_hash_path(&self.workdir),
            content_fingerprint(content).to_string(),
        );
        Ok(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_and_load_daily() {
        let dir = tempdir().unwrap();
        let store = FsMemoryStore::new(dir.path());
        store
            .append_daily_log("## Decisions\n\nShip it.\n")
            .unwrap();
        let loaded = store.load_recent().unwrap();
        assert!(loaded.contains("Ship it"));
    }

    #[test]
    fn load_active_agent_notes() {
        let dir = tempdir().unwrap();
        let active_dir = dir.path().join(".zene").join("notes").join("active");
        std::fs::create_dir_all(&active_dir).unwrap();
        std::fs::write(
            active_dir.join("001-prefix-invariants.md"),
            "System prefix must be frozen for prompt caching.",
        )
        .unwrap();

        let store = FsMemoryStore::new(dir.path());
        let loaded = store.load_recent().unwrap();
        assert!(loaded.contains("Invariant Note"));
        assert!(loaded.contains("System prefix must be frozen"));
    }
}
