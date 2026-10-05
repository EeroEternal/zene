//! Local SQLite session store.
//!
//! One `sessions` table whose row holds the serialized session JSON; a
//! single-file alternative to the per-session JSON files of
//! [`crate::FileSessionStore`].

use std::path::Path;
use std::sync::Mutex;

use anyhow::{Context, Result};
use rusqlite::{params, Connection};

use crate::{parse_session_raw, SessionRecord, SessionStore};

pub struct SqliteSessionStore {
    conn: Mutex<Connection>,
}

impl std::fmt::Debug for SqliteSessionStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SqliteSessionStore").finish_non_exhaustive()
    }
}

impl SqliteSessionStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path).context("open session sqlite database")?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                json TEXT NOT NULL,
                updated_at TEXT NOT NULL
            )",
        )
        .context("create sessions table")?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl SessionStore for SqliteSessionStore {
    fn save(&self, session: &SessionRecord) -> Result<()> {
        let raw = serde_json::to_string(session).context("serialize session")?;
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT INTO sessions (id, json, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json, updated_at = excluded.updated_at",
            params![session.meta.id, raw, session.meta.updated_at.to_rfc3339()],
        )
        .context("upsert session")?;
        Ok(())
    }

    fn load(&self, id: &str) -> Result<Option<SessionRecord>> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn
            .prepare_cached("SELECT json FROM sessions WHERE id = ?1")
            .context("prepare session select")?;
        let mut rows = stmt.query(params![id]).context("query session")?;
        let Some(row) = rows.next().context("read session row")? else {
            return Ok(None);
        };
        let raw: String = row.get(0).context("read session json")?;
        parse_session_raw(&raw, Some(id))
            .map(Some)
            .context("parse stored session")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn save_load_roundtrip_and_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = SqliteSessionStore::open(dir.path().join("sessions.db")).unwrap();
        assert!(store.load("missing").unwrap().is_none());

        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(zene_llm::Message::user("hi"));
        store.save(&session).unwrap();
        let loaded = store.load(&session.meta.id).unwrap().expect("saved");
        assert_eq!(loaded.messages, session.messages);

        // Upsert replaces, not duplicates.
        session.push_message(zene_llm::Message::assistant("hello"));
        store.save(&session).unwrap();
        let reloaded = store.load(&session.meta.id).unwrap().expect("updated");
        assert_eq!(reloaded.messages, session.messages);
    }
}
