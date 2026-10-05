//! Compaction checkpoints for rewind / fork (aligned with grok compaction_checkpoints).

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{session_record_dir, CompactionEntry, SessionEvent, SessionRecord, TodoItem};
use zene_llm::Message;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCheckpoint {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub reason: String,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub events: Vec<SessionEvent>,
    #[serde(default)]
    pub event_sequence: u64,
    pub todos: Vec<TodoItem>,
    pub compactions: Vec<CompactionEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_usage: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens_used: Option<u32>,
}

impl SessionCheckpoint {
    pub fn from_session(session: &SessionRecord, reason: &str) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            reason: reason.to_string(),
            messages: session.messages.clone(),
            events: session.events.clone(),
            event_sequence: session.event_sequence,
            todos: session.todos.clone(),
            compactions: session.compactions.clone(),
            context_window_usage: session.context_window_usage,
            context_tokens_used: session.context_tokens_used,
        }
    }
}

pub fn checkpoints_dir(session_id: &str) -> PathBuf {
    session_record_dir(session_id).join("compaction_checkpoints")
}

/// Retention cap: snapshots are full-session copies, so only the newest few
/// are kept per session.
/// ponytail: pruning by file mtime (no content reads); raise if `/rewind`
/// ever needs deeper history.
const MAX_CHECKPOINTS: usize = 5;

fn write_atomic(path: &Path, raw: &str) -> Result<()> {
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, raw).with_context(|| format!("write {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("rename {}", path.display()))?;
    Ok(())
}

/// Best-effort retention: drop all but the newest [`MAX_CHECKPOINTS`].
/// `keep` (the just-written checkpoint, also `LATEST`) is never pruned.
fn prune_checkpoints(dir: &Path, keep: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        files.push((modified, path));
    }
    if files.len() <= MAX_CHECKPOINTS {
        return;
    }
    files.sort_by(|a, b| {
        let a_keep = a.1.file_stem().and_then(|s| s.to_str()) == Some(keep);
        let b_keep = b.1.file_stem().and_then(|s| s.to_str()) == Some(keep);
        b_keep.cmp(&a_keep).then_with(|| b.0.cmp(&a.0))
    });
    for (_, path) in files.into_iter().skip(MAX_CHECKPOINTS) {
        let _ = fs::remove_file(path);
    }
}

pub fn save_checkpoint(session: &SessionRecord, reason: &str) -> Result<SessionCheckpoint> {
    let dir = checkpoints_dir(&session.meta.id);
    fs::create_dir_all(&dir).context("create compaction_checkpoints dir")?;
    let checkpoint = SessionCheckpoint::from_session(session, reason);
    let path = dir.join(format!("{}.json", checkpoint.id));
    let raw = serde_json::to_string(&checkpoint).context("serialize checkpoint")?;
    write_atomic(&path, &raw)?;

    // Keep a pointer to the latest checkpoint for `/rewind`.
    let latest = dir.join("LATEST");
    fs::write(&latest, checkpoint.id.as_bytes()).context("write LATEST checkpoint pointer")?;
    prune_checkpoints(&dir, &checkpoint.id);
    Ok(checkpoint)
}

pub fn load_checkpoint(session_id: &str, checkpoint_id: &str) -> Result<SessionCheckpoint> {
    let path = checkpoints_dir(session_id).join(format!("{checkpoint_id}.json"));
    let raw =
        fs::read_to_string(&path).with_context(|| format!("read checkpoint {}", path.display()))?;
    serde_json::from_str(&raw).context("parse checkpoint")
}

pub fn latest_checkpoint_id(session_id: &str) -> Result<Option<String>> {
    let path = checkpoints_dir(session_id).join("LATEST");
    if !path.exists() {
        return Ok(None);
    }
    let id = fs::read_to_string(&path).context("read LATEST checkpoint")?;
    let id = id.trim();
    if id.is_empty() {
        Ok(None)
    } else {
        Ok(Some(id.to_string()))
    }
}

pub fn restore_checkpoint(session: &mut SessionRecord, checkpoint: &SessionCheckpoint) {
    session.messages = checkpoint.messages.clone();
    session.todos = checkpoint.todos.clone();
    session.compactions = checkpoint.compactions.clone();
    session.context_window_usage = checkpoint.context_window_usage;
    session.context_tokens_used = checkpoint.context_tokens_used;
    session.record_rewound_with_target(
        &checkpoint.id,
        Some(checkpoint.event_sequence),
        Some(checkpoint.messages.clone()),
    );
    session.event_sequence = session.event_sequence.max(checkpoint.event_sequence);
    session.meta.updated_at = Utc::now();
}

/// Fork a session into a new id, copying messages/todos/compactions.
pub fn fork_session(session: &SessionRecord, workdir: &Path) -> SessionRecord {
    let mut forked = SessionRecord::new(workdir);
    forked.meta.title = format!("{} (fork)", session.meta.title);
    forked.meta.parent_session_id = Some(session.meta.id.clone());
    forked.meta.parent_sequence = Some(session.event_sequence);
    forked.messages = session.messages.clone();
    forked.events = session.events.clone();
    forked.event_sequence = session.event_sequence;
    forked.todos = session.todos.clone();
    forked.compactions = session.compactions.clone();
    forked.context_window_usage = session.context_window_usage;
    forked.context_tokens_used = session.context_tokens_used;
    forked.record_branch_forked(&session.meta.id, &forked.meta.id.clone());
    forked.record_branch_summary(
        &forked.meta.id.clone(),
        &format!(
            "Forked from session {} at event {}",
            session.meta.id, session.event_sequence
        ),
    );
    forked
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionRecord;
    use std::env;
    use std::path::Path;

    #[test]
    fn checkpoint_roundtrip() {
        let _guard = crate::ZENE_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = env::var("ZENE_HOME").ok();
        env::set_var("ZENE_HOME", dir.path());
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("sys");
        session.push_message(zene_llm::Message::user("hi"));
        let cp = save_checkpoint(&session, "test").expect("save");
        session.push_message(zene_llm::Message::assistant("later"));
        let loaded = load_checkpoint(&session.meta.id, &cp.id).expect("load");
        assert_eq!(loaded.messages.len(), 2);
        restore_checkpoint(&mut session, &loaded);
        assert_eq!(session.messages.len(), 2);
        assert!(
            matches!(session.events.last(), Some(SessionEvent::Rewound { checkpoint_id, .. }) if checkpoint_id == &cp.id)
        );
        match prev {
            Some(v) => env::set_var("ZENE_HOME", v),
            None => env::remove_var("ZENE_HOME"),
        }
    }

    #[test]
    fn checkpoint_pruning_keeps_recent() {
        let _guard = crate::ZENE_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = env::var("ZENE_HOME").ok();
        env::set_var("ZENE_HOME", dir.path());
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("sys");
        for _ in 0..(MAX_CHECKPOINTS + 2) {
            save_checkpoint(&session, "test").expect("save");
        }
        let kept = std::fs::read_dir(checkpoints_dir(&session.meta.id))
            .unwrap()
            .flatten()
            .filter(|e| e.path().extension().and_then(|e| e.to_str()) == Some("json"))
            .count();
        assert_eq!(kept, MAX_CHECKPOINTS);
        // LATEST must always survive pruning and be loadable.
        let latest = latest_checkpoint_id(&session.meta.id)
            .unwrap()
            .expect("latest checkpoint");
        load_checkpoint(&session.meta.id, &latest).expect("load latest");
        match prev {
            Some(v) => env::set_var("ZENE_HOME", v),
            None => env::remove_var("ZENE_HOME"),
        }
    }
}
