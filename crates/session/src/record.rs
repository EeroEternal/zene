use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use zene_config::sessions_dir;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RecordEntry {
    TurnPrompt {
        turn_id: String,
        prompt: String,
        ts: DateTime<Utc>,
    },
    StepBegin {
        turn_id: String,
        step_id: String,
        step: u32,
        ts: DateTime<Utc>,
    },
    ToolCall {
        name: String,
        arguments: String,
        ts: DateTime<Utc>,
    },
    ToolResult {
        name: String,
        content: String,
        is_error: bool,
        ts: DateTime<Utc>,
    },
    TurnEnd {
        turn_id: String,
        steps: u32,
        ts: DateTime<Utc>,
    },
    Compaction {
        reason: String,
        compacted_count: usize,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_before: Option<u32>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tokens_after: Option<u32>,
        ts: DateTime<Utc>,
    },
    Error {
        message: String,
        ts: DateTime<Utc>,
    },
}

#[derive(Debug, Clone)]
pub struct AgentRecordWriter {
    path: PathBuf,
}

impl AgentRecordWriter {
    pub fn for_session(session_id: &str) -> Result<Self> {
        let dir = session_record_dir(session_id);
        fs::create_dir_all(&dir)
            .with_context(|| format!("create session record dir: {}", dir.display()))?;
        Ok(Self {
            path: record_path(session_id),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, entry: &RecordEntry) -> Result<()> {
        let line = serde_json::to_string(entry).context("serialize record entry")?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open record file: {}", self.path.display()))?;
        writeln!(file, "{line}").context("append record entry")?;
        Ok(())
    }

    pub fn read_all(&self) -> Result<Vec<RecordEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&self.path)
            .with_context(|| format!("open record file: {}", self.path.display()))?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();
        for line in reader.lines() {
            let line = line.context("read record line")?;
            if line.trim().is_empty() {
                continue;
            }
            let entry: RecordEntry =
                serde_json::from_str(&line).context("parse record entry")?;
            entries.push(entry);
        }
        Ok(entries)
    }
}

pub fn session_record_dir(session_id: &str) -> PathBuf {
    sessions_dir().join(session_id)
}

pub fn record_path(session_id: &str) -> PathBuf {
    session_record_dir(session_id).join("record.jsonl")
}

pub fn export_session(session_id: &str, output: &Path) -> Result<()> {
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    let session_file = super::session_path(session_id);
    let record_file = record_path(session_id);

    if !session_file.exists() {
        anyhow::bail!("session not found: {}", session_id);
    }

    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).context("create export output parent dir")?;
    }

    let file = fs::File::create(output)
        .with_context(|| format!("create export zip: {}", output.display()))?;
    let mut zip = ZipWriter::new(file);
    let options = SimpleFileOptions::default();

    let session_name = format!("{session_id}.json");
    zip.start_file(&session_name, options)
        .context("start session zip entry")?;
    let mut session_bytes = fs::read(&session_file).context("read session file")?;
    zip.write_all(&session_bytes)
        .context("write session to zip")?;

    if record_file.exists() {
        zip.start_file("record.jsonl", options)
            .context("start record zip entry")?;
        session_bytes = fs::read(&record_file).context("read record file")?;
        zip.write_all(&session_bytes)
            .context("write record to zip")?;
    }

    zip.finish().context("finalize export zip")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;

    fn with_temp_home<F: FnOnce()>(test: F) {
        static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = LOCK.lock().expect("test lock");
        let temp = tempfile::tempdir().expect("tempdir");
        let prev = env::var("ZENE_HOME").ok();
        env::set_var("ZENE_HOME", temp.path());
        test();
        match prev {
            Some(value) => env::set_var("ZENE_HOME", value),
            None => env::remove_var("ZENE_HOME"),
        }
    }

    #[test]
    fn append_and_read_roundtrip() {
        with_temp_home(|| {
            let writer = AgentRecordWriter::for_session("test-session").expect("writer");
            let ts = Utc::now();
            let entries = vec![
                RecordEntry::TurnPrompt {
                    turn_id: "turn-1".into(),
                    prompt: "hello".into(),
                    ts,
                },
                RecordEntry::StepBegin {
                    turn_id: "turn-1".into(),
                    step_id: "step-1".into(),
                    step: 1,
                    ts,
                },
                RecordEntry::ToolCall {
                    name: "Read".into(),
                    arguments: r#"{"path":"foo.rs"}"#.into(),
                    ts,
                },
                RecordEntry::TurnEnd {
                    turn_id: "turn-1".into(),
                    steps: 1,
                    ts,
                },
            ];
            for entry in &entries {
                writer.append(entry).expect("append");
            }
            let loaded = writer.read_all().expect("read");
            assert_eq!(loaded.len(), 4);
            assert!(matches!(loaded[0], RecordEntry::TurnPrompt { .. }));
            assert!(matches!(loaded[3], RecordEntry::TurnEnd { .. }));
        });
    }

    #[test]
    fn export_creates_zip_with_expected_files() {
        with_temp_home(|| {
            let session_id = "export-test";
            let session_file = super::super::session_path(session_id);
            fs::create_dir_all(sessions_dir()).expect("sessions dir");
            fs::write(
                &session_file,
                r#"{
                    "meta": {
                        "id": "export-test",
                        "title": "New session",
                        "workdir": ".",
                        "created_at": "2026-01-01T00:00:00Z",
                        "updated_at": "2026-01-01T00:00:00Z"
                    },
                    "messages": [],
                    "compactions": [],
                    "todos": [
                        { "id": "t1", "content": "Exported todo", "status": "pending" }
                    ]
                }"#,
            )
            .expect("session");

            let writer = AgentRecordWriter::for_session(session_id).expect("writer");
            writer
                .append(&RecordEntry::TurnPrompt {
                    turn_id: "t1".into(),
                    prompt: "hi".into(),
                    ts: Utc::now(),
                })
                .expect("append");

            let output = tempfile::NamedTempFile::new().expect("output");
            export_session(session_id, output.path()).expect("export");

            let file = fs::File::open(output.path()).expect("open zip");
            let mut archive = zip::ZipArchive::new(file).expect("zip archive");
            assert!(archive.by_name("export-test.json").is_ok());
            assert!(archive.by_name("record.jsonl").is_ok());

            let mut session_entry = archive.by_name("export-test.json").expect("session entry");
            let mut session_json = String::new();
            std::io::Read::read_to_string(&mut session_entry, &mut session_json)
                .expect("read session zip entry");
            assert!(session_json.contains("Exported todo"));
        });
    }
}
