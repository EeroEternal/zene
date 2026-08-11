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

pub fn save_checkpoint(session: &SessionRecord, reason: &str) -> Result<SessionCheckpoint> {
    let dir = checkpoints_dir(&session.meta.id);
    fs::create_dir_all(&dir).context("create compaction_checkpoints dir")?;
    let checkpoint = SessionCheckpoint::from_session(session, reason);
    let path = dir.join(format!("{}.json", checkpoint.id));
    let raw = serde_json::to_string_pretty(&checkpoint).context("serialize checkpoint")?;
    fs::write(&path, raw).with_context(|| format!("write checkpoint {}", path.display()))?;

    // Keep a pointer to the latest checkpoint for `/rewind`.
    let latest = dir.join("LATEST");
    fs::write(&latest, checkpoint.id.as_bytes()).context("write LATEST checkpoint pointer")?;
    Ok(checkpoint)
}

pub fn load_checkpoint(session_id: &str, checkpoint_id: &str) -> Result<SessionCheckpoint> {
    let path = checkpoints_dir(session_id).join(format!("{checkpoint_id}.json"));
    let raw = fs::read_to_string(&path)
        .with_context(|| format!("read checkpoint {}", path.display()))?;
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

pub fn list_checkpoints(session_id: &str) -> Result<Vec<SessionCheckpoint>> {
    let dir = checkpoints_dir(session_id);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(&dir).context("read checkpoints dir")? {
        let entry = entry.context("read checkpoint entry")?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let raw = fs::read_to_string(&path).context("read checkpoint file")?;
        if let Ok(cp) = serde_json::from_str::<SessionCheckpoint>(&raw) {
            out.push(cp);
        }
    }
    out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(out)
}

pub fn restore_checkpoint(session: &mut SessionRecord, checkpoint: &SessionCheckpoint) {
    session.messages = checkpoint.messages.clone();
    session.events = checkpoint.events.clone();
    session.todos = checkpoint.todos.clone();
    session.compactions = checkpoint.compactions.clone();
    session.context_window_usage = checkpoint.context_window_usage;
    session.context_tokens_used = checkpoint.context_tokens_used;
    session.meta.updated_at = Utc::now();
}

/// Fork a session into a new id, copying messages/todos/compactions.
pub fn fork_session(session: &SessionRecord, workdir: &Path) -> SessionRecord {
    let mut forked = SessionRecord::new(workdir);
    forked.meta.title = format!("{} (fork)", session.meta.title);
    forked.messages = session.messages.clone();
    forked.events = session.events.clone();
    forked.todos = session.todos.clone();
    forked.compactions = session.compactions.clone();
    forked.context_window_usage = session.context_window_usage;
    forked.context_tokens_used = session.context_tokens_used;
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
        match prev {
            Some(v) => env::set_var("ZENE_HOME", v),
            None => env::remove_var("ZENE_HOME"),
        }
    }
}
