mod paths;
mod checkpoint;
mod record;
mod todo;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zene_llm::Message;

pub use paths::{sessions_dir, workdir_slug, zene_home};
pub use checkpoint::{
    fork_session, latest_checkpoint_id, list_checkpoints, load_checkpoint, restore_checkpoint,
    save_checkpoint, SessionCheckpoint,
};
pub use todo::{TodoItem, TodoStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub workdir: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionEntry {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub summary: String,
    pub compacted_message_count: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub meta: SessionMeta,
    pub messages: Vec<Message>,
    #[serde(default)]
    pub compactions: Vec<CompactionEntry>,
    #[serde(default)]
    pub todos: Vec<TodoItem>,
    /// Last observed context occupancy percent (0..=100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_window_usage: Option<u8>,
    /// Last observed effective prompt tokens used for water-level checks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_tokens_used: Option<u32>,
}

impl SessionRecord {
    pub fn new(workdir: &Path) -> Self {
        let now = Utc::now();
        Self {
            meta: SessionMeta {
                id: Uuid::new_v4().to_string(),
                title: "New session".to_string(),
                workdir: workdir.display().to_string(),
                created_at: now,
                updated_at: now,
            },
            messages: Vec::new(),
            compactions: Vec::new(),
            todos: Vec::new(),
            context_window_usage: None,
            context_tokens_used: None,
        }
    }

    pub fn update_context_usage(&mut self, tokens_used: u32, context_window: u32) {
        self.context_tokens_used = Some(tokens_used);
        if context_window > 0 {
            let pct = ((u64::from(tokens_used) * 100) / u64::from(context_window)).min(100) as u8;
            self.context_window_usage = Some(pct);
        }
        self.meta.updated_at = Utc::now();
    }

    pub fn push_message(&mut self, message: Message) {
        if message.role == zene_llm::Role::System
            && self.messages.iter().any(|m| m.role == zene_llm::Role::System)
        {
            return;
        }
        self.messages.push(message);
        self.meta.updated_at = Utc::now();
    }

    pub fn ensure_system_message(&mut self, content: &str) {
        if self
            .messages
            .first()
            .is_some_and(|message| message.role == zene_llm::Role::System)
        {
            return;
        }
        self.messages.insert(0, Message::system(content));
        self.meta.updated_at = Utc::now();
    }

    /// Replace the leading system message, or insert one if missing.
    pub fn set_system_message(&mut self, content: &str) {
        if let Some(message) = self.messages.first_mut() {
            if message.role == zene_llm::Role::System {
                message.content = Some(content.to_string());
                self.meta.updated_at = Utc::now();
                return;
            }
        }
        self.messages.insert(0, Message::system(content));
        self.meta.updated_at = Utc::now();
    }

    pub fn set_title_from_prompt(&mut self, prompt: &str) {
        if self.meta.title == "New session" {
            let title = prompt.lines().next().unwrap_or(prompt);
            self.meta.title = title.chars().take(60).collect();
        }
    }

    /// Replace older messages with a compaction summary, keeping system + recent tail.
    pub fn apply_compaction(&mut self, summary: String, compacted_count: usize) -> CompactionEntry {
        self.record_compaction_event(
            "llm_summarize",
            compacted_count,
            Some(summary),
            None,
            None,
        )
    }

    pub fn record_compaction_event(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
    ) -> CompactionEntry {
        let entry = CompactionEntry {
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            summary: summary.unwrap_or_default(),
            compacted_message_count: compacted_count,
            reason: Some(reason.to_string()),
            tokens_before,
            tokens_after,
        };
        self.compactions.push(entry.clone());
        self.meta.updated_at = Utc::now();
        entry
    }

    pub fn replace_messages_after_compaction(
        &mut self,
        summary: String,
        tail_start: usize,
        compacted_count: usize,
    ) {
        self.replace_messages_after_compaction_with_stats(
            summary,
            tail_start,
            compacted_count,
            "llm_summarize",
            None,
            None,
        );
    }

    pub fn replace_messages_after_compaction_with_stats(
        &mut self,
        summary: String,
        tail_start: usize,
        compacted_count: usize,
        reason: &str,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
    ) {
        let system = self
            .messages
            .first()
            .filter(|m| m.role == zene_llm::Role::System)
            .cloned();
        let tail = self.messages[tail_start..].to_vec();
        let summary_message = Message::compaction_summary(format!(
            "[Previous conversation summary]\n{summary}"
        ));

        self.messages.clear();
        if let Some(system) = system {
            self.messages.push(system);
        }
        self.messages.push(summary_message);
        self.messages.extend(tail);
        self.record_compaction_event(
            reason,
            compacted_count,
            Some(summary),
            tokens_before,
            tokens_after,
        );
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(sessions_dir()).context("create sessions dir")?;
        let path = session_path(&self.meta.id);
        let raw = serde_json::to_string_pretty(self).context("serialize session")?;
        fs::write(path, raw).context("write session file")?;
        Ok(())
    }

    pub fn load(id: &str) -> Result<Self> {
        let path = session_path(id);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read session file: {}", path.display()))?;
        parse_session_raw(&raw, Some(id)).context("parse session file")
    }
}

pub fn session_path(id: &str) -> PathBuf {
    sessions_dir().join(format!("{id}.json"))
}

/// Parse a session JSON document, migrating older shapes that lack `meta`.
pub fn parse_session_raw(raw: &str, fallback_id: Option<&str>) -> Result<SessionRecord> {
    if let Ok(record) = serde_json::from_str::<SessionRecord>(raw) {
        return Ok(record);
    }

    let value: serde_json::Value =
        serde_json::from_str(raw).context("session file is not valid JSON")?;

    // Legacy: top-level messages array (no wrapper object).
    if let Some(messages) = value.as_array() {
        let messages: Vec<Message> =
            serde_json::from_value(serde_json::Value::Array(messages.clone()))
                .context("parse legacy session message array")?;
        return Ok(legacy_record(fallback_id, None, None, messages));
    }

    // Legacy: object with messages but no meta envelope.
    if value.get("meta").is_none() && value.get("messages").is_some() {
        let messages: Vec<Message> = serde_json::from_value(
            value
                .get("messages")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .context("parse legacy session messages")?;
        let id = value
            .get("id")
            .and_then(|v| v.as_str())
            .or(fallback_id)
            .map(str::to_string);
        let title = value
            .get("title")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let workdir = value
            .get("workdir")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let mut record = legacy_record(id.as_deref(), title.as_deref(), workdir.as_deref(), messages);
        if let Some(compactions) = value.get("compactions") {
            if let Ok(parsed) = serde_json::from_value::<Vec<CompactionEntry>>(compactions.clone()) {
                record.compactions = parsed;
            }
        }
        if let Some(todos) = value.get("todos") {
            if let Ok(parsed) = serde_json::from_value::<Vec<TodoItem>>(todos.clone()) {
                record.todos = parsed;
            }
        }
        return Ok(record);
    }

    Err(anyhow::anyhow!(
        "unsupported session file shape (expected SessionRecord with meta)"
    ))
}

fn legacy_record(
    id: Option<&str>,
    title: Option<&str>,
    workdir: Option<&str>,
    messages: Vec<Message>,
) -> SessionRecord {
    let now = Utc::now();
    let inferred_title = messages
        .iter()
        .find(|m| m.role == zene_llm::Role::User)
        .and_then(|m| m.content.as_deref())
        .map(|prompt| {
            prompt
                .lines()
                .next()
                .unwrap_or(prompt)
                .chars()
                .take(60)
                .collect::<String>()
        });
    SessionRecord {
        meta: SessionMeta {
            id: id
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| Uuid::new_v4().to_string()),
            title: title
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .or(inferred_title)
                .unwrap_or_else(|| "Recovered session".to_string()),
            workdir: workdir.unwrap_or(".").to_string(),
            created_at: now,
            updated_at: now,
        },
        messages,
        compactions: Vec::new(),
        todos: Vec::new(),
        context_window_usage: None,
        context_tokens_used: None,
    }
}

pub fn list_sessions_for_workdir(workdir: &Path) -> Result<Vec<SessionMeta>> {
    fs::create_dir_all(sessions_dir()).context("create sessions dir")?;
    let slug = workdir_slug(workdir);
    let mut sessions = Vec::new();
    for entry in fs::read_dir(sessions_dir()).context("read sessions dir")? {
        let entry = entry.context("read session entry")?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }
        let raw = match fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(err) => {
                eprintln!(
                    "skipping unreadable session file {}: {err}",
                    path.display()
                );
                continue;
            }
        };
        let fallback_id = path.file_stem().and_then(|s| s.to_str());
        let record = match parse_session_raw(&raw, fallback_id) {
            Ok(record) => record,
            Err(err) => {
                // One corrupt/legacy file must not break Sessions UI / ACP session/list.
                eprintln!(
                    "skipping unreadable session file {}: {err:#}",
                    path.display()
                );
                continue;
            }
        };
        if workdir_slug(Path::new(&record.meta.workdir)) == slug {
            sessions.push(record.meta);
        }
    }
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

pub fn ensure_zene_home() -> Result<()> {
    fs::create_dir_all(zene_home()).context("create zene home")?;
    fs::create_dir_all(sessions_dir()).context("create sessions dir")?;
    Ok(())
}

pub use record::{export_session, record_path, session_record_dir, AgentRecordWriter, RecordEntry};

#[cfg(test)]
pub(crate) static ZENE_HOME_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn ensure_system_message_is_idempotent() {
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("system prompt");
        session.ensure_system_message("system prompt");
        assert_eq!(session.messages.len(), 1);
        assert_eq!(session.messages[0].content.as_deref(), Some("system prompt"));
    }

    #[test]
    fn push_message_skips_duplicate_system() {
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("system prompt");
        session.push_message(Message::system("another system"));
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn compaction_replaces_prefix_and_records_history() {
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("system prompt");
        session.push_message(Message::user("old request"));
        session.push_message(Message::assistant("old reply"));
        session.push_message(Message::user("recent request"));
        session.push_message(Message::assistant("recent reply"));

        session.replace_messages_after_compaction(
            "summarized old work".into(),
            3,
            2,
        );

        assert_eq!(session.messages.len(), 4);
        assert_eq!(session.messages[0].content.as_deref(), Some("system prompt"));
        assert_eq!(
            session.messages[1].kind,
            Some(zene_llm::MessageKind::CompactionSummary)
        );
        assert!(session.messages[1]
            .content
            .as_deref()
            .unwrap()
            .contains("summarized old work"));
        assert_eq!(session.messages[2].content.as_deref(), Some("recent request"));
        assert_eq!(session.compactions.len(), 1);
        assert_eq!(session.compactions[0].compacted_message_count, 2);
    }

    #[test]
    fn compaction_session_roundtrip_serialization() {
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("sys");
        session.replace_messages_after_compaction("summary".into(), 1, 0);

        let raw = serde_json::to_string(&session).expect("serialize");
        let loaded: SessionRecord = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(loaded.compactions.len(), 1);
        assert_eq!(loaded.compactions[0].summary, "summary");
    }

    #[test]
    fn todos_session_roundtrip_serialization() {
        let mut session = SessionRecord::new(Path::new("."));
        session.todos.push(TodoItem {
            id: "task-1".into(),
            content: "Ship persistence".into(),
            status: TodoStatus::InProgress,
        });

        let raw = serde_json::to_string(&session).expect("serialize");
        let loaded: SessionRecord = serde_json::from_str(&raw).expect("deserialize");
        assert_eq!(loaded.todos.len(), 1);
        assert_eq!(loaded.todos[0].id, "task-1");
        assert_eq!(loaded.todos[0].content, "Ship persistence");
        assert_eq!(loaded.todos[0].status, TodoStatus::InProgress);
    }

    #[test]
    fn old_session_without_todos_defaults_empty() {
        let raw = r#"{
            "meta": {
                "id": "legacy",
                "title": "New session",
                "workdir": ".",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            },
            "messages": [],
            "compactions": []
        }"#;
        let session: SessionRecord = serde_json::from_str(raw).expect("deserialize");
        assert!(session.todos.is_empty());
    }

    #[test]
    fn parses_legacy_session_without_meta_envelope() {
        let raw = r#"{
            "id": "old-1",
            "title": "Old chat",
            "workdir": "/tmp/project",
            "messages": [
                {"role": "user", "content": "hello"}
            ]
        }"#;
        let session = parse_session_raw(raw, Some("old-1")).expect("parse legacy");
        assert_eq!(session.meta.id, "old-1");
        assert_eq!(session.meta.title, "Old chat");
        assert_eq!(session.meta.workdir, "/tmp/project");
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn list_sessions_skips_corrupt_files() {
        let _guard = ZENE_HOME_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ZENE_HOME", dir.path());
        let sessions = sessions_dir();
        fs::create_dir_all(&sessions).unwrap();

        let good = SessionRecord::new(Path::new("/tmp/project"));
        good.save().unwrap();

        fs::write(sessions.join("broken.json"), "{\"messages\":").unwrap();
        fs::write(
            sessions.join("legacy.json"),
            r#"{"id":"legacy","workdir":"/tmp/project","messages":[{"role":"user","content":"hi"}]}"#,
        )
        .unwrap();

        let listed = list_sessions_for_workdir(Path::new("/tmp/project")).expect("list");
        assert!(listed.iter().any(|m| m.id == good.meta.id));
        assert!(listed.iter().any(|m| m.id == "legacy"));
        std::env::remove_var("ZENE_HOME");
    }
}
