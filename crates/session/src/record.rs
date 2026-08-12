use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use crate::paths::sessions_dir;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionCheckpointState {
    TurnStarted,
    /// A previously interrupted model-boundary turn was explicitly resumed.
    TurnResumed,
    StepStarted,
    ToolStarted,
    ToolCompleted,
    TurnCompleted,
    TurnCancelled,
    RuntimeShutdown,
    RuntimeFailed,
    Failed,
}

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
    /// Marks execution boundaries that were invalidated by a session rewind.
    Rewound {
        checkpoint_id: String,
        target_sequence: Option<u64>,
        ts: DateTime<Utc>,
    },
    /// Durable execution boundary used for recovery and tool-call idempotency.
    #[serde(rename = "execution_checkpoint")]
    ExecutionCheckpoint {
        turn_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        step_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_call_id: Option<String>,
        state: ExecutionCheckpointState,
        idempotency_key: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_epoch: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_request_hash: Option<String>,
        ts: DateTime<Utc>,
    },
}

/// Explicit recovery decision and the reason automatic execution is or is not safe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryPlan {
    pub disposition: RecoveryDisposition,
    pub active_turns: Vec<RecoveryExecution>,
    pub active_tools: Vec<RecoveryExecution>,
    /// Policy gate for a future explicit model-boundary resume command.
    pub safe_resume_allowed: bool,
    /// Automatic restart is intentionally disabled until resume execution exists.
    pub automatic_resume_implemented: bool,
    pub reason: String,
}

impl RecoverySnapshot {
    pub fn plan(&self) -> RecoveryPlan {
        let disposition = self.disposition();
        let (safe_resume_allowed, reason) = match disposition {
            RecoveryDisposition::Clean => (false, "no incomplete execution"),
            RecoveryDisposition::AlreadyCompleted => (false, "latest execution is terminal"),
            RecoveryDisposition::SafeToResume => (true, "only a model-boundary turn is open"),
            RecoveryDisposition::RequiresToolInspection => (
                false,
                "a tool side effect has no durable completion boundary",
            ),
            RecoveryDisposition::RequiresManualIntervention => (
                false,
                "runtime or execution failure requires operator review",
            ),
        };
        RecoveryPlan {
            disposition,
            active_turns: self.active_turns.clone(),
            active_tools: self.active_tools.clone(),
            safe_resume_allowed,
            automatic_resume_implemented: false,
            reason: reason.to_string(),
        }
    }
}

/// An execution boundary that remains open in the durable record.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ResumeCandidate {
    pub turn_id: String,
    pub prompt: String,
    pub context_epoch: Option<u64>,
    pub model_request_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RecoveryExecution {
    pub checkpoint_index: usize,
    pub turn_id: String,
    pub step_id: Option<String>,
    pub tool_call_id: Option<String>,
    pub state: ExecutionCheckpointState,
    pub idempotency_key: String,
    pub context_epoch: Option<u64>,
    pub model_request_hash: Option<String>,
    pub ts: DateTime<Utc>,
}

/// Conservative recovery decision derived from durable execution boundaries.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryDisposition {
    /// No record exists that requires recovery handling.
    Clean,
    /// The last execution ended in a terminal state.
    AlreadyCompleted,
    /// A turn is open, but no tool side effect is awaiting inspection.
    SafeToResume,
    /// A tool started without a durable completion boundary.
    RequiresToolInspection,
    /// Runtime or execution failure requires explicit operator handling.
    RequiresManualIntervention,
}

/// Read-only recovery view reconstructed from execution checkpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RecoverySnapshot {
    pub latest_checkpoint: Option<RecordEntry>,
    pub active_turns: Vec<RecoveryExecution>,
    pub active_tools: Vec<RecoveryExecution>,
    pub latest_runtime_checkpoint: Option<RecordEntry>,
    pub latest_runtime_state: Option<ExecutionCheckpointState>,
    /// Original prompts for open turns, recovered from durable TurnPrompt entries.
    pub resume_candidates: Vec<ResumeCandidate>,
}

impl RecoverySnapshot {
    /// Classify this snapshot without starting or replaying any execution.
    pub fn disposition(&self) -> RecoveryDisposition {
        let latest_runtime_failed = self.latest_runtime_state
            == Some(ExecutionCheckpointState::RuntimeFailed)
            && self.latest_runtime_checkpoint == self.latest_checkpoint;
        if latest_runtime_failed
            || self.latest_checkpoint.as_ref().is_some_and(|entry| {
                matches!(
                    entry,
                    RecordEntry::ExecutionCheckpoint {
                        state: ExecutionCheckpointState::Failed,
                        ..
                    }
                )
            })
        {
            return RecoveryDisposition::RequiresManualIntervention;
        }
        if !self.active_tools.is_empty() {
            return RecoveryDisposition::RequiresToolInspection;
        }
        if !self.active_turns.is_empty() {
            return RecoveryDisposition::SafeToResume;
        }
        match self.latest_checkpoint.as_ref() {
            None => RecoveryDisposition::Clean,
            Some(RecordEntry::ExecutionCheckpoint { state, .. }) => match state {
                ExecutionCheckpointState::TurnCompleted
                | ExecutionCheckpointState::TurnCancelled => RecoveryDisposition::AlreadyCompleted,
                ExecutionCheckpointState::RuntimeShutdown => RecoveryDisposition::Clean,
                ExecutionCheckpointState::Failed | ExecutionCheckpointState::RuntimeFailed => {
                    RecoveryDisposition::RequiresManualIntervention
                }
                _ => RecoveryDisposition::Clean,
            },
            Some(_) => RecoveryDisposition::Clean,
        }
    }

    pub fn has_incomplete_execution(&self) -> bool {
        !self.active_turns.is_empty() || !self.active_tools.is_empty()
    }

    pub fn requires_inspection(&self) -> bool {
        !matches!(
            self.disposition(),
            RecoveryDisposition::Clean | RecoveryDisposition::AlreadyCompleted
        )
    }
}

#[derive(Debug, Clone)]
pub struct AgentRecordWriter {
    path: PathBuf,
}

impl AgentRecordWriter {
    pub fn for_session(session_id: &str) -> Result<Self> {
        Self::from_path(record_path(session_id))
    }

    /// Construct a writer at an explicit path, primarily for injected stores and tests.
    pub fn from_path(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)
                .with_context(|| format!("create session record dir: {}", dir.display()))?;
        }
        Ok(Self { path })
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

    /// Append an execution checkpoint once per idempotency key.
    ///
    /// Runtime writes are serialized by the owning Agent/Runtime, but the
    /// read-before-append guard also protects replay paths from duplicating a
    /// completed tool boundary.
    pub fn append_execution_checkpoint(&self, checkpoint: &RecordEntry) -> Result<bool> {
        let RecordEntry::ExecutionCheckpoint {
            idempotency_key, ..
        } = checkpoint
        else {
            anyhow::bail!("expected execution checkpoint record");
        };
        if self
            .read_all()?
            .iter()
            .any(|entry| matches!(entry, RecordEntry::ExecutionCheckpoint { idempotency_key: existing, .. } if existing == idempotency_key))
        {
            return Ok(false);
        }
        self.append(checkpoint)?;
        Ok(true)
    }

    /// Atomically claim a safe model-boundary resume candidate.
    ///
    /// The claim marker is created with `create_new`, so concurrent or stale
    /// runtime instances cannot both claim the same candidate. The recovery
    /// view is re-read after claiming to ensure the candidate is still the
    /// current open execution. A marker is intentionally retained after a
    /// failed write: refusing a potentially duplicate resume is safer than
    /// replaying a model request after an uncertain crash.
    pub fn claim_safe_resume(&self, candidate: &ResumeCandidate) -> Result<bool> {
        let fence_dir = self
            .path
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("resume-fences");
        fs::create_dir_all(&fence_dir).with_context(|| {
            format!("create resume fence dir: {}", fence_dir.display())
        })?;
        let fence_path = fence_dir.join(format!("{}.claim", resume_fence_key(candidate)));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&fence_path)
        {
            Ok(mut fence) => {
                writeln!(
                    fence,
                    "turn_id={} claimed_at={}",
                    candidate.turn_id,
                    Utc::now().to_rfc3339()
                )
                .context("write resume fence")?;
                fence.sync_all().context("persist resume fence")?;
            }
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(err) => {
                return Err(err).with_context(|| {
                    format!("create resume fence: {}", fence_path.display())
                });
            }
        }

        let snapshot = self.recovery_snapshot()?;
        let current = if snapshot.disposition() == RecoveryDisposition::SafeToResume
            && snapshot.active_tools.is_empty()
            && snapshot.active_turns.len() == 1
        {
            snapshot.resume_candidates.into_iter().next()
        } else {
            None
        };
        if current.as_ref() != Some(candidate) {
            return Ok(false);
        }

        let checkpoint = RecordEntry::ExecutionCheckpoint {
            turn_id: candidate.turn_id.clone(),
            step_id: None,
            tool_call_id: None,
            state: ExecutionCheckpointState::TurnResumed,
            idempotency_key: format!("resume/{}", resume_fence_key(candidate)),
            context_epoch: candidate.context_epoch,
            model_request_hash: candidate.model_request_hash.clone(),
            ts: Utc::now(),
        };
        self.append_execution_checkpoint(&checkpoint)
    }

    /// Return all durable execution checkpoints in record order.
    pub fn execution_checkpoints(&self) -> Result<Vec<RecordEntry>> {
        Ok(self
            .read_all()?
            .into_iter()
            .filter(|entry| matches!(entry, RecordEntry::ExecutionCheckpoint { .. }))
            .collect())
    }

    /// Reconstruct a read-only recovery view from durable execution boundaries.
    ///
    /// This method never replays a turn or tool. Callers must explicitly decide whether
    /// an open boundary is safe to inspect, resume, or abandon.
    pub fn recovery_snapshot(&self) -> Result<RecoverySnapshot> {
        let mut snapshot = RecoverySnapshot::default();
        let mut active_turns: HashMap<String, RecoveryExecution> = HashMap::new();
        let mut active_tools: HashMap<String, RecoveryExecution> = HashMap::new();
        let mut prompts: HashMap<String, String> = HashMap::new();

        for (index, entry) in self.read_all()?.into_iter().enumerate() {
            if matches!(entry, RecordEntry::Rewound { .. }) {
                // A rewind invalidates every open execution boundary from the
                // abandoned path. Later checkpoints, if any, rebuild state.
                active_turns.clear();
                active_tools.clear();
                snapshot.latest_checkpoint = None;
                snapshot.latest_runtime_checkpoint = None;
                snapshot.latest_runtime_state = None;
                continue;
            }
            if let RecordEntry::TurnPrompt {
                turn_id, prompt, ..
            } = &entry
            {
                prompts.insert(turn_id.clone(), prompt.clone());
                continue;
            }
            let RecordEntry::ExecutionCheckpoint {
                turn_id,
                step_id,
                tool_call_id,
                state,
                idempotency_key,
                context_epoch,
                model_request_hash,
                ts,
            } = &entry
            else {
                continue;
            };
            snapshot.latest_checkpoint = Some(entry.clone());
            if matches!(
                state,
                ExecutionCheckpointState::RuntimeShutdown | ExecutionCheckpointState::RuntimeFailed
            ) {
                snapshot.latest_runtime_checkpoint = Some(entry.clone());
                snapshot.latest_runtime_state = Some(state.clone());
            }
            let execution = RecoveryExecution {
                checkpoint_index: index,
                turn_id: turn_id.clone(),
                step_id: step_id.clone(),
                tool_call_id: tool_call_id.clone(),
                state: state.clone(),
                idempotency_key: idempotency_key.clone(),
                context_epoch: *context_epoch,
                model_request_hash: model_request_hash.clone(),
                ts: *ts,
            };
            match state {
                ExecutionCheckpointState::TurnStarted => {
                    active_turns.insert(turn_id.clone(), execution);
                }
                ExecutionCheckpointState::ToolStarted => {
                    if let Some(tool_call_id) = tool_call_id {
                        active_tools.insert(tool_call_id.clone(), execution);
                    }
                }
                ExecutionCheckpointState::ToolCompleted => {
                    if let Some(tool_call_id) = tool_call_id {
                        active_tools.remove(tool_call_id);
                    }
                }
                ExecutionCheckpointState::TurnCompleted
                | ExecutionCheckpointState::TurnCancelled
                | ExecutionCheckpointState::Failed => {
                    if let Some(tool_call_id) = tool_call_id {
                        active_tools.remove(tool_call_id);
                    } else {
                        active_turns.remove(turn_id);
                    }
                }
                ExecutionCheckpointState::RuntimeShutdown
                | ExecutionCheckpointState::RuntimeFailed
                | ExecutionCheckpointState::StepStarted => {}
                ExecutionCheckpointState::TurnResumed => {
                    active_turns.remove(turn_id);
                }
            }
        }

        snapshot.active_turns = active_turns.into_values().collect();
        snapshot.active_tools = active_tools.into_values().collect();
        snapshot
            .active_turns
            .sort_by_key(|item| item.checkpoint_index);
        snapshot.resume_candidates = snapshot
            .active_turns
            .iter()
            .filter_map(|execution| {
                prompts
                    .get(&execution.turn_id)
                    .map(|prompt| ResumeCandidate {
                        turn_id: execution.turn_id.clone(),
                        prompt: prompt.clone(),
                        context_epoch: execution.context_epoch,
                        model_request_hash: execution.model_request_hash.clone(),
                    })
            })
            .collect();
        snapshot
            .active_tools
            .sort_by_key(|item| item.checkpoint_index);
        Ok(snapshot)
    }

    /// Backward-compatible flat view of open turn/tool boundaries.
    pub fn resume_candidate(&self) -> Result<Option<ResumeCandidate>> {
        let snapshot = self.recovery_snapshot()?;
        if snapshot.disposition() != RecoveryDisposition::SafeToResume {
            return Ok(None);
        }
        Ok(snapshot.resume_candidates.into_iter().next())
    }

    pub fn incomplete_execution_checkpoints(&self) -> Result<Vec<RecordEntry>> {
        let snapshot = self.recovery_snapshot()?;
        let mut pending = snapshot
            .active_turns
            .into_iter()
            .chain(snapshot.active_tools)
            .map(|execution| {
                (
                    execution.checkpoint_index,
                    RecordEntry::ExecutionCheckpoint {
                        turn_id: execution.turn_id,
                        step_id: execution.step_id,
                        tool_call_id: execution.tool_call_id,
                        state: execution.state,
                        idempotency_key: execution.idempotency_key,
                        context_epoch: execution.context_epoch,
                        model_request_hash: execution.model_request_hash,
                        ts: execution.ts,
                    },
                )
            })
            .collect::<Vec<_>>();
        pending.sort_by_key(|(index, _)| *index);
        Ok(pending.into_iter().map(|(_, entry)| entry).collect())
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
            let entry: RecordEntry = serde_json::from_str(&line).context("parse record entry")?;
            entries.push(entry);
        }
        Ok(entries)
    }
}

fn resume_fence_key(candidate: &ResumeCandidate) -> String {
    // Keep the durable fence name stable across processes and restarts. The
    // standard library's hasher is not a persistence format, so use a small
    // deterministic FNV-1a digest over the candidate identity instead.
    let identity = format!(
        "{}\0{}\0{:?}\0{:?}",
        candidate.turn_id,
        candidate.prompt,
        candidate.context_epoch,
        candidate.model_request_hash
    );
    let mut hash = 0xcbf29ce484222325u64;
    for byte in identity.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
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
        let _guard = crate::ZENE_HOME_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
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
                RecordEntry::ExecutionCheckpoint {
                    turn_id: "turn-1".into(),
                    step_id: Some("step-1".into()),
                    tool_call_id: Some("call-1".into()),
                    state: ExecutionCheckpointState::ToolCompleted,
                    idempotency_key: "turn-1/step-1/call-1/completed".into(),
                    context_epoch: Some(2),
                    model_request_hash: Some("hash".into()),
                    ts,
                },
            ];
            for entry in &entries {
                writer.append(entry).expect("append");
            }
            let loaded = writer.read_all().expect("read");
            assert_eq!(loaded.len(), 5);
            assert!(matches!(loaded[0], RecordEntry::TurnPrompt { .. }));
            assert!(matches!(loaded[3], RecordEntry::TurnEnd { .. }));
            assert!(matches!(loaded[4], RecordEntry::ExecutionCheckpoint { .. }));
        });
    }

    #[test]
    fn recovery_snapshot_groups_open_boundaries_and_runtime_state() {
        with_temp_home(|| {
            let writer = AgentRecordWriter::for_session("recovery-test").expect("writer");
            let ts = Utc::now();
            let turn =
                |state: ExecutionCheckpointState, key: &str| RecordEntry::ExecutionCheckpoint {
                    turn_id: "turn-1".into(),
                    step_id: None,
                    tool_call_id: None,
                    state,
                    idempotency_key: key.to_string(),
                    context_epoch: None,
                    model_request_hash: None,
                    ts,
                };
            writer
                .append(&RecordEntry::TurnPrompt {
                    turn_id: "turn-1".into(),
                    prompt: "resume me".into(),
                    ts,
                })
                .unwrap();
            writer
                .append(&turn(ExecutionCheckpointState::TurnStarted, "turn/start"))
                .unwrap();
            let candidate = writer.resume_candidate().unwrap().expect("candidate");
            assert_eq!(candidate.turn_id, "turn-1");
            assert_eq!(candidate.prompt, "resume me");
            writer
                .append(&turn(ExecutionCheckpointState::TurnCompleted, "turn/end"))
                .unwrap();
            writer
                .append(&RecordEntry::ExecutionCheckpoint {
                    turn_id: "turn-2".into(),
                    step_id: Some("step-1".into()),
                    tool_call_id: Some("call-1".into()),
                    state: ExecutionCheckpointState::ToolStarted,
                    idempotency_key: "tool/start".into(),
                    context_epoch: None,
                    model_request_hash: None,
                    ts,
                })
                .unwrap();
            writer
                .append(&RecordEntry::ExecutionCheckpoint {
                    turn_id: "turn-2".into(),
                    step_id: None,
                    tool_call_id: None,
                    state: ExecutionCheckpointState::RuntimeFailed,
                    idempotency_key: "runtime/failed".into(),
                    context_epoch: None,
                    model_request_hash: None,
                    ts,
                })
                .unwrap();
            let snapshot = writer.recovery_snapshot().unwrap();
            assert_eq!(snapshot.active_turns.len(), 0);
            assert_eq!(snapshot.active_tools.len(), 1);
            assert_eq!(
                snapshot.active_tools[0].tool_call_id.as_deref(),
                Some("call-1")
            );
            assert_eq!(
                snapshot.latest_runtime_state,
                Some(ExecutionCheckpointState::RuntimeFailed)
            );
            assert_eq!(
                snapshot.disposition(),
                RecoveryDisposition::RequiresManualIntervention
            );
            assert!(snapshot.requires_inspection());
            assert_eq!(writer.incomplete_execution_checkpoints().unwrap().len(), 1);
        });
    }

    #[test]
    fn recovery_disposition_prioritizes_tool_and_turn_states() {
        let base = RecoverySnapshot::default();
        assert_eq!(base.disposition(), RecoveryDisposition::Clean);

        let completed = RecoverySnapshot {
            latest_checkpoint: Some(RecordEntry::ExecutionCheckpoint {
                turn_id: "turn".into(),
                step_id: None,
                tool_call_id: None,
                state: ExecutionCheckpointState::TurnCompleted,
                idempotency_key: "turn/completed".into(),
                context_epoch: None,
                model_request_hash: None,
                ts: Utc::now(),
            }),
            ..RecoverySnapshot::default()
        };
        assert_eq!(
            completed.disposition(),
            RecoveryDisposition::AlreadyCompleted
        );

        let turn = RecoveryExecution {
            checkpoint_index: 1,
            turn_id: "turn".into(),
            step_id: None,
            tool_call_id: None,
            state: ExecutionCheckpointState::TurnStarted,
            idempotency_key: "turn/started".into(),
            context_epoch: None,
            model_request_hash: None,
            ts: Utc::now(),
        };
        let tool = RecoveryExecution {
            tool_call_id: Some("call".into()),
            ..turn.clone()
        };
        assert_eq!(
            RecoverySnapshot {
                active_turns: vec![turn.clone()],
                ..RecoverySnapshot::default()
            }
            .disposition(),
            RecoveryDisposition::SafeToResume
        );
        assert_eq!(
            RecoverySnapshot {
                active_turns: vec![turn],
                active_tools: vec![tool],
                ..RecoverySnapshot::default()
            }
            .disposition(),
            RecoveryDisposition::RequiresToolInspection
        );
    }

    #[test]
    fn rewind_boundary_clears_open_recovery_state() {
        with_temp_home(|| {
            let writer = AgentRecordWriter::for_session("rewind-recovery").expect("writer");
            let ts = Utc::now();
            writer
                .append(&RecordEntry::ExecutionCheckpoint {
                    turn_id: "turn-1".into(),
                    step_id: Some("step-1".into()),
                    tool_call_id: Some("call-1".into()),
                    state: ExecutionCheckpointState::ToolStarted,
                    idempotency_key: "tool/start".into(),
                    context_epoch: None,
                    model_request_hash: None,
                    ts,
                })
                .unwrap();
            writer
                .append(&RecordEntry::Rewound {
                    checkpoint_id: "cp-1".into(),
                    target_sequence: Some(3),
                    ts,
                })
                .unwrap();
            let snapshot = writer.recovery_snapshot().unwrap();
            assert!(snapshot.active_turns.is_empty());
            assert!(snapshot.active_tools.is_empty());
            assert_eq!(snapshot.disposition(), RecoveryDisposition::Clean);
            assert!(!snapshot.plan().safe_resume_allowed);
            assert!(!snapshot.plan().automatic_resume_implemented);
        });
    }

    #[test]
    fn recovery_plan_explains_safe_and_unsafe_states() {
        let safe = RecoverySnapshot {
            active_turns: vec![RecoveryExecution {
                checkpoint_index: 1,
                turn_id: "turn".into(),
                step_id: Some("step".into()),
                tool_call_id: None,
                state: ExecutionCheckpointState::TurnStarted,
                idempotency_key: "turn/start".into(),
                context_epoch: None,
                model_request_hash: None,
                ts: Utc::now(),
            }],
            ..RecoverySnapshot::default()
        };
        assert!(safe.plan().safe_resume_allowed);
        assert!(!safe.plan().automatic_resume_implemented);
        assert_eq!(safe.plan().reason, "only a model-boundary turn is open");

        let unsafe_plan = RecoverySnapshot {
            active_tools: vec![RecoveryExecution {
                tool_call_id: Some("call".into()),
                ..safe.active_turns[0].clone()
            }],
            ..safe
        };
        assert!(!unsafe_plan.plan().safe_resume_allowed);
        assert!(unsafe_plan.plan().reason.contains("tool side effect"));
    }

    #[test]
    fn execution_checkpoint_append_is_idempotent() {
        with_temp_home(|| {
            let writer = AgentRecordWriter::for_session("checkpoint-test").expect("writer");
            let checkpoint = RecordEntry::ExecutionCheckpoint {
                turn_id: "turn-1".into(),
                step_id: Some("step-1".into()),
                tool_call_id: Some("call-1".into()),
                state: ExecutionCheckpointState::ToolStarted,
                idempotency_key: "turn-1/step-1/call-1/started".into(),
                context_epoch: None,
                model_request_hash: None,
                ts: Utc::now(),
            };
            assert!(writer
                .append_execution_checkpoint(&checkpoint)
                .expect("append"));
            assert!(!writer
                .append_execution_checkpoint(&checkpoint)
                .expect("dedupe"));
            assert_eq!(writer.read_all().expect("read").len(), 1);
        });
    }

    #[test]
    fn safe_resume_claim_accepts_once_and_rejects_duplicate_or_stale_claims() {
        with_temp_home(|| {
            let writer = AgentRecordWriter::for_session("resume-fence-test").expect("writer");
            let ts = Utc::now();
            writer
                .append(&RecordEntry::TurnPrompt {
                    turn_id: "turn-1".into(),
                    prompt: "resume me".into(),
                    ts,
                })
                .expect("prompt");
            writer
                .append(&RecordEntry::ExecutionCheckpoint {
                    turn_id: "turn-1".into(),
                    step_id: None,
                    tool_call_id: None,
                    state: ExecutionCheckpointState::TurnStarted,
                    idempotency_key: "turn-1/started".into(),
                    context_epoch: Some(3),
                    model_request_hash: Some("request-hash".into()),
                    ts,
                })
                .expect("turn checkpoint");

            let candidate = writer.resume_candidate().expect("candidate").expect("safe");
            assert!(writer.claim_safe_resume(&candidate).expect("first claim"));
            assert!(!writer.claim_safe_resume(&candidate).expect("duplicate claim"));
            assert!(writer.resume_candidate().expect("recovery read").is_none());

            let stale = ResumeCandidate {
                model_request_hash: Some("different-request".into()),
                ..candidate
            };
            assert!(!writer.claim_safe_resume(&stale).expect("stale claim"));
            assert_eq!(
                writer
                    .execution_checkpoints()
                    .expect("checkpoints")
                    .iter()
                    .filter(|entry| matches!(
                        entry,
                        RecordEntry::ExecutionCheckpoint {
                            state: ExecutionCheckpointState::TurnResumed,
                            ..
                        }
                    ))
                    .count(),
                1
            );
        });
    }

    #[test]
    fn safe_resume_claim_is_serialized_for_concurrent_writers() {
        with_temp_home(|| {
            let writer = AgentRecordWriter::for_session("resume-concurrent-test").expect("writer");
            let ts = Utc::now();
            writer
                .append(&RecordEntry::TurnPrompt {
                    turn_id: "turn-1".into(),
                    prompt: "resume me".into(),
                    ts,
                })
                .expect("prompt");
            writer
                .append(&RecordEntry::ExecutionCheckpoint {
                    turn_id: "turn-1".into(),
                    step_id: None,
                    tool_call_id: None,
                    state: ExecutionCheckpointState::TurnStarted,
                    idempotency_key: "turn-1/started".into(),
                    context_epoch: None,
                    model_request_hash: None,
                    ts,
                })
                .expect("turn checkpoint");
            let candidate = writer.resume_candidate().expect("candidate").expect("safe");
            let writers = [writer.clone(), writer.clone()];
            let candidates = [candidate.clone(), candidate];
            let results = std::thread::scope(|scope| {
                writers
                    .into_iter()
                    .zip(candidates)
                    .map(|(writer, candidate)| {
                        scope.spawn(move || writer.claim_safe_resume(&candidate).expect("claim"))
                    })
                    .map(|thread| thread.join().expect("claim thread"))
                    .collect::<Vec<_>>()
            });
            assert_eq!(results.iter().filter(|claimed| **claimed).count(), 1);
            assert_eq!(
                writer
                    .execution_checkpoints()
                    .expect("checkpoints")
                    .iter()
                    .filter(|entry| matches!(
                        entry,
                        RecordEntry::ExecutionCheckpoint {
                            state: ExecutionCheckpointState::TurnResumed,
                            ..
                        }
                    ))
                    .count(),
                1
            );
        });
    }

    #[test]
    fn legacy_execution_checkpoint_without_optional_fields_remains_readable() {
        with_temp_home(|| {
            let writer = AgentRecordWriter::for_session("legacy-checkpoint-test").expect("writer");
            let raw = r#"{"type":"execution_checkpoint","turn_id":"legacy-turn","state":"turn_started","idempotency_key":"legacy/start","ts":"2026-01-01T00:00:00Z"}"#;
            fs::write(writer.path(), format!("{raw}\n")).expect("legacy record");
            let entries = writer.read_all().expect("read legacy record");
            assert!(matches!(
                entries.as_slice(),
                [RecordEntry::ExecutionCheckpoint {
                    turn_id,
                    step_id: None,
                    tool_call_id: None,
                    context_epoch: None,
                    model_request_hash: None,
                    ..
                }] if turn_id == "legacy-turn"
            ));
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
