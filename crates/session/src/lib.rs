mod checkpoint;
mod paths;
mod record;
mod todo;

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zene_llm::Message;

/// Current event-backed conversation schema. Older records may omit this field.
pub const CURRENT_CONVERSATION_SCHEMA_VERSION: u16 = 1;

pub use checkpoint::{
    fork_session, latest_checkpoint_id, list_checkpoints, load_checkpoint, restore_checkpoint,
    save_checkpoint, SessionCheckpoint,
};
pub use paths::{sessions_dir, workdir_slug, zene_home};
pub use todo::{TodoItem, TodoStatus};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: String,
    pub title: String,
    pub workdir: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Parent session for a forked conversation. Optional for legacy sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    /// Last parent event included when this session was forked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_sequence: Option<u64>,
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

/// Identity of an append-only conversation fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConversationEventIdentity {
    pub id: String,
    pub sequence: u64,
}

/// Conversation facts dual-written beside the materialized message cache.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionEvent {
    MessageAppended {
        #[serde(default)]
        sequence: u64,
        id: String,
        created_at: DateTime<Utc>,
        message: Message,
    },
    SystemPrefixChanged {
        #[serde(default)]
        sequence: u64,
        id: String,
        created_at: DateTime<Utc>,
        content: String,
    },
    CompactionApplied {
        #[serde(default)]
        sequence: u64,
        id: String,
        created_at: DateTime<Utc>,
        entry: CompactionEntry,
        /// Snapshot of the materialized conversation after compaction. Older
        /// sessions omit this and use the legacy replay fallback.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        messages_after: Option<Vec<Message>>,
    },
    TurnStarted {
        #[serde(default)]
        sequence: u64,
        id: String,
        turn_id: String,
        created_at: DateTime<Utc>,
        prompt: String,
    },
    StepStarted {
        #[serde(default)]
        sequence: u64,
        id: String,
        turn_id: String,
        step_id: String,
        created_at: DateTime<Utc>,
        step: u32,
    },
    TurnEnded {
        #[serde(default)]
        sequence: u64,
        id: String,
        turn_id: String,
        created_at: DateTime<Utc>,
        steps: u32,
        status: String,
    },
    Checkpoint {
        #[serde(default)]
        sequence: u64,
        id: String,
        turn_id: Option<String>,
        step_id: Option<String>,
        tool_call_id: Option<String>,
        created_at: DateTime<Utc>,
        state: String,
        idempotency_key: String,
    },
    ToolCall {
        #[serde(default)]
        sequence: u64,
        id: String,
        turn_id: Option<String>,
        step_id: Option<String>,
        created_at: DateTime<Utc>,
        name: String,
        arguments: String,
    },
    ToolResult {
        #[serde(default)]
        sequence: u64,
        id: String,
        turn_id: Option<String>,
        step_id: Option<String>,
        created_at: DateTime<Utc>,
        name: String,
        content: String,
        is_error: bool,
        duration_ms: Option<u64>,
    },
    PermissionDecision {
        #[serde(default)]
        sequence: u64,
        id: String,
        turn_id: Option<String>,
        step_id: Option<String>,
        tool_call_id: String,
        created_at: DateTime<Utc>,
        tool_name: String,
        allowed: bool,
    },
    ModeChanged {
        #[serde(default)]
        sequence: u64,
        id: String,
        created_at: DateTime<Utc>,
        mode_id: String,
    },
    ModelChanged {
        #[serde(default)]
        sequence: u64,
        id: String,
        created_at: DateTime<Utc>,
        model: String,
    },
    BranchForked {
        #[serde(default)]
        sequence: u64,
        id: String,
        created_at: DateTime<Utc>,
        source_session_id: String,
        branch_session_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_sequence: Option<u64>,
    },
    Rewound {
        #[serde(default)]
        sequence: u64,
        id: String,
        created_at: DateTime<Utc>,
        checkpoint_id: String,
        /// Event sequence represented by the restored checkpoint. Older
        /// sessions omit this and retain snapshot-only compatibility.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target_sequence: Option<u64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        messages_after: Option<Vec<Message>>,
    },
}

/// Why a session view had to use the materialized message cache.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectionFallbackReason {
    NoEvents,
    LegacyCompactionWithoutSnapshot,
    LegacyRewindWithoutSnapshot,
    IncompleteEventLog,
}

impl ProjectionFallbackReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NoEvents => "no_events",
            Self::LegacyCompactionWithoutSnapshot => "legacy_compaction_without_snapshot",
            Self::LegacyRewindWithoutSnapshot => "legacy_rewind_without_snapshot",
            Self::IncompleteEventLog => "incomplete_event_log",
        }
    }
}

/// Read-only conversation projection reconstructed from the active event path.
#[derive(Debug, Clone)]
pub struct SessionView {
    pub messages: Vec<Message>,
    /// Complete append-only history retained for inspection and recovery.
    pub events: Vec<SessionEvent>,
    /// Events on the active path, including the shared parent history and
    /// branch-local suffix. This is explicit even when it currently equals the
    /// complete history for a materialized fork.
    pub active_events: Vec<SessionEvent>,
    pub active_branch_id: Option<String>,
    pub active_path_start_sequence: Option<u64>,
    pub source_event_count: usize,
    /// Whether the compatibility `messages` cache differs from the event projection.
    pub cache_drift_detected: bool,
    pub used_materialized_fallback: bool,
    pub fallback_reason: Option<ProjectionFallbackReason>,
}

impl SessionView {
    pub fn from_events(events: &[SessionEvent], fallback: &[Message]) -> Self {
        Self::from_events_for_session(events, fallback, None)
    }

    /// Return the event-backed projection or an error when compatibility fallback
    /// would be required. Legacy callers should continue using [`Self::from_events`].
    pub fn try_from_events(
        events: &[SessionEvent],
        fallback: &[Message],
        session_id: Option<&str>,
    ) -> std::result::Result<Self, ProjectionFallbackReason> {
        let view = Self::from_events_for_session(events, fallback, session_id);
        if view.used_materialized_fallback {
            Err(view
                .fallback_reason
                .unwrap_or(ProjectionFallbackReason::IncompleteEventLog))
        } else {
            Ok(view)
        }
    }

    pub fn from_events_for_session(
        events: &[SessionEvent],
        fallback: &[Message],
        session_id: Option<&str>,
    ) -> Self {
        let branch = session_id.and_then(|session_id| {
            events.iter().rev().find_map(|event| match event {
                SessionEvent::BranchForked {
                    branch_session_id,
                    parent_sequence,
                    sequence,
                    ..
                } if branch_session_id == session_id => Some((
                    branch_session_id.clone(),
                    parent_sequence.unwrap_or(*sequence),
                    *sequence,
                )),
                _ => None,
            })
        });
        let (active_branch_id, active_path_start_sequence) = branch
            .as_ref()
            .map(|(branch_id, parent, _)| (Some(branch_id.clone()), Some(*parent)))
            .unwrap_or((None, None));
        let branch_events: Vec<SessionEvent> = match branch {
            Some((_, parent_sequence, fork_sequence)) => events
                .iter()
                .filter(|event| {
                    let sequence = event.sequence();
                    sequence <= parent_sequence || sequence >= fork_sequence
                })
                .cloned()
                .collect(),
            None => events.to_vec(),
        };
        let rewind = branch_events.iter().rev().find_map(|event| match event {
            SessionEvent::Rewound {
                sequence,
                target_sequence: Some(target_sequence),
                ..
            } => Some((*sequence, *target_sequence)),
            _ => None,
        });
        let active_events: Vec<SessionEvent> = match rewind {
            Some((rewind_sequence, target_sequence)) => branch_events
                .into_iter()
                .filter(|event| {
                    let sequence = event.sequence();
                    sequence <= target_sequence || sequence >= rewind_sequence
                })
                .collect(),
            None => branch_events,
        };
        let mut messages = Vec::new();
        let mut has_message_fact = false;
        let mut fallback_reason = None;
        for event in &active_events {
            match event {
                SessionEvent::MessageAppended { message, .. } => {
                    has_message_fact = true;
                    messages.push(message.clone());
                }
                SessionEvent::SystemPrefixChanged { content, .. } => {
                    has_message_fact = true;
                    if let Some(system) = messages
                        .first_mut()
                        .filter(|message| message.role == zene_llm::Role::System)
                    {
                        system.content = Some(content.clone());
                    } else {
                        messages.insert(0, Message::system(content));
                    }
                }
                SessionEvent::CompactionApplied {
                    messages_after: Some(snapshot),
                    ..
                } => {
                    has_message_fact = true;
                    fallback_reason = None;
                    messages = snapshot.clone();
                }
                SessionEvent::CompactionApplied {
                    messages_after: None,
                    ..
                } => {
                    fallback_reason =
                        Some(ProjectionFallbackReason::LegacyCompactionWithoutSnapshot);
                }
                SessionEvent::Rewound {
                    messages_after: Some(snapshot),
                    ..
                } => {
                    has_message_fact = true;
                    fallback_reason = None;
                    messages = snapshot.clone();
                }
                SessionEvent::Rewound {
                    messages_after: None,
                    ..
                } => {
                    fallback_reason = Some(ProjectionFallbackReason::LegacyRewindWithoutSnapshot);
                }
                _ => {}
            }
        }
        if events.is_empty() {
            if !fallback.is_empty() {
                fallback_reason = Some(ProjectionFallbackReason::NoEvents);
            }
        } else if !has_message_fact && fallback_reason.is_none() {
            fallback_reason = Some(ProjectionFallbackReason::IncompleteEventLog);
        }
        let cache_drift_detected =
            serde_json::to_vec(&messages).ok() != serde_json::to_vec(fallback).ok();
        let used_materialized_fallback = fallback_reason.is_some();
        if used_materialized_fallback {
            messages = fallback.to_vec();
        }
        Self {
            messages,
            events: events.to_vec(),
            active_events,
            active_branch_id,
            active_path_start_sequence,
            source_event_count: events.len(),
            cache_drift_detected,
            used_materialized_fallback,
            fallback_reason,
        }
    }
}

/// Persistence boundary for mutable session state.
///
/// The runtime owns [`SessionRecord`] in memory; implementations decide where
/// snapshots are written. Conversation semantics remain independent of the
/// backing store.
pub trait SessionStore: Send + Sync {
    fn save(&self, session: &SessionRecord) -> Result<()>;
}

/// Default on-disk session store used by the compatibility APIs.
#[derive(Debug, Clone, Copy, Default)]
pub struct FileSessionStore;

impl SessionStore for FileSessionStore {
    fn save(&self, session: &SessionRecord) -> Result<()> {
        fs::create_dir_all(sessions_dir()).context("create sessions dir")?;
        let path = session_path(&session.meta.id);
        let raw = serde_json::to_string_pretty(session).context("serialize session")?;
        fs::write(path, raw).context("write session file")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub meta: SessionMeta,
    pub messages: Vec<Message>,
    /// Version of the conversation event log. `0` identifies pre-event or
    /// legacy records that may require one-time migration.
    #[serde(default)]
    pub conversation_schema_version: u16,
    /// Materialized compatibility cache; events are the future conversation SoT.
    #[serde(default)]
    pub events: Vec<SessionEvent>,
    /// Monotonic sequence for conversation events in this session.
    #[serde(default)]
    pub event_sequence: u64,
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

impl SessionEvent {
    fn with_sequence(mut self, sequence: u64) -> Self {
        match &mut self {
            Self::MessageAppended {
                sequence: value, ..
            }
            | Self::SystemPrefixChanged {
                sequence: value, ..
            }
            | Self::CompactionApplied {
                sequence: value, ..
            }
            | Self::TurnStarted {
                sequence: value, ..
            }
            | Self::StepStarted {
                sequence: value, ..
            }
            | Self::TurnEnded {
                sequence: value, ..
            }
            | Self::Checkpoint {
                sequence: value, ..
            }
            | Self::ToolCall {
                sequence: value, ..
            }
            | Self::ToolResult {
                sequence: value, ..
            }
            | Self::PermissionDecision {
                sequence: value, ..
            }
            | Self::ModeChanged {
                sequence: value, ..
            }
            | Self::ModelChanged {
                sequence: value, ..
            }
            | Self::BranchForked {
                sequence: value, ..
            }
            | Self::Rewound {
                sequence: value, ..
            } => *value = sequence,
        }
        self
    }

    fn sequence(&self) -> u64 {
        match self {
            Self::MessageAppended { sequence, .. }
            | Self::SystemPrefixChanged { sequence, .. }
            | Self::CompactionApplied { sequence, .. }
            | Self::TurnStarted { sequence, .. }
            | Self::StepStarted { sequence, .. }
            | Self::TurnEnded { sequence, .. }
            | Self::Checkpoint { sequence, .. }
            | Self::ToolCall { sequence, .. }
            | Self::ToolResult { sequence, .. }
            | Self::PermissionDecision { sequence, .. }
            | Self::ModeChanged { sequence, .. }
            | Self::ModelChanged { sequence, .. }
            | Self::BranchForked { sequence, .. }
            | Self::Rewound { sequence, .. } => *sequence,
        }
    }
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
                parent_session_id: None,
                parent_sequence: None,
            },
            messages: Vec::new(),
            conversation_schema_version: CURRENT_CONVERSATION_SCHEMA_VERSION,
            events: Vec::new(),
            event_sequence: 0,
            compactions: Vec::new(),
            todos: Vec::new(),
            context_window_usage: None,
            context_tokens_used: None,
        }
    }

    pub fn is_event_backed(&self) -> bool {
        self.conversation_schema_version >= CURRENT_CONVERSATION_SCHEMA_VERSION
    }

    /// Migrate a legacy materialized session into the event-backed format.
    ///
    /// Migration is explicit and idempotent. It is committed to `self` only
    /// when the candidate can be projected without materialized fallback.
    /// Legacy compaction, rewind, and incomplete event logs therefore remain
    /// legacy records until a future migration can reconstruct their facts.
    pub fn migrate_to_event_backed(&mut self) -> bool {
        if self.is_event_backed() {
            return false;
        }

        let mut candidate = self.clone();
        candidate.normalize_event_sequence();
        if candidate.events.is_empty() {
            let messages = candidate.messages.clone();
            for message in messages {
                candidate.append_event(SessionEvent::MessageAppended {
                    sequence: 0,
                    id: Uuid::new_v4().to_string(),
                    created_at: Utc::now(),
                    message,
                });
            }
        }

        let view = SessionView::from_events_for_session(
            &candidate.events,
            &candidate.messages,
            Some(&candidate.meta.id),
        );
        if view.used_materialized_fallback {
            return false;
        }

        candidate.conversation_schema_version = CURRENT_CONVERSATION_SCHEMA_VERSION;
        candidate.meta.updated_at = Utc::now();
        *self = candidate;
        true
    }

    pub fn update_context_usage(&mut self, tokens_used: u32, context_window: u32) {
        self.context_tokens_used = Some(tokens_used);
        if context_window > 0 {
            let pct = ((u64::from(tokens_used) * 100) / u64::from(context_window)).min(100) as u8;
            self.context_window_usage = Some(pct);
        }
        self.meta.updated_at = Utc::now();
    }

    fn append_event(&mut self, event: SessionEvent) {
        if self.events.iter().any(|event| event.sequence() == 0) {
            self.normalize_event_sequence();
        } else {
            self.event_sequence = self.event_sequence.max(
                self.events
                    .iter()
                    .map(SessionEvent::sequence)
                    .max()
                    .unwrap_or(0),
            );
        }
        self.event_sequence = self.event_sequence.saturating_add(1);
        self.events.push(event.with_sequence(self.event_sequence));
    }

    fn normalize_event_sequence(&mut self) {
        let mut sequence = self.event_sequence;
        for event in &mut self.events {
            if event.sequence() == 0 {
                sequence = sequence.saturating_add(1);
                *event = event.clone().with_sequence(sequence);
            } else {
                sequence = sequence.max(event.sequence());
            }
        }
        self.event_sequence = sequence;
    }

    pub fn push_message(&mut self, message: Message) {
        if message.role == zene_llm::Role::System
            && self
                .messages
                .iter()
                .any(|m| m.role == zene_llm::Role::System)
        {
            return;
        }
        self.messages.push(message.clone());
        self.append_event(SessionEvent::MessageAppended {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            message,
        });
        self.meta.updated_at = Utc::now();
    }

    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    pub fn view(&self) -> SessionView {
        SessionView::from_events_for_session(&self.events, &self.messages, Some(&self.meta.id))
    }

    /// Strict event-backed view for new integrations. Legacy compatibility
    /// callers should use [`Self::view`] and inspect its fallback reason.
    pub fn try_view(&self) -> std::result::Result<SessionView, ProjectionFallbackReason> {
        SessionView::try_from_events(&self.events, &self.messages, Some(&self.meta.id))
    }

    pub fn record_turn_started(
        &mut self,
        turn_id: &str,
        prompt: &str,
    ) -> ConversationEventIdentity {
        let id = Uuid::new_v4().to_string();
        self.append_event(SessionEvent::TurnStarted {
            sequence: 0,
            id: id.clone(),
            turn_id: turn_id.to_string(),
            created_at: Utc::now(),
            prompt: prompt.to_string(),
        });
        self.meta.updated_at = Utc::now();
        ConversationEventIdentity {
            id,
            sequence: self.event_sequence,
        }
    }

    pub fn record_step_started(
        &mut self,
        turn_id: &str,
        step_id: &str,
        step: u32,
    ) -> ConversationEventIdentity {
        let id = Uuid::new_v4().to_string();
        self.append_event(SessionEvent::StepStarted {
            sequence: 0,
            id: id.clone(),
            turn_id: turn_id.to_string(),
            step_id: step_id.to_string(),
            created_at: Utc::now(),
            step,
        });
        self.meta.updated_at = Utc::now();
        ConversationEventIdentity {
            id,
            sequence: self.event_sequence,
        }
    }

    pub fn record_turn_ended(&mut self, turn_id: &str, steps: u32, status: &str) {
        self.append_event(SessionEvent::TurnEnded {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            turn_id: turn_id.to_string(),
            created_at: Utc::now(),
            steps,
            status: status.to_string(),
        });
        self.meta.updated_at = Utc::now();
    }

    pub fn record_checkpoint(
        &mut self,
        turn_id: Option<&str>,
        step_id: Option<&str>,
        tool_call_id: Option<&str>,
        state: &str,
        idempotency_key: &str,
    ) -> ConversationEventIdentity {
        let id = Uuid::new_v4().to_string();
        self.append_event(SessionEvent::Checkpoint {
            sequence: 0,
            id: id.clone(),
            turn_id: turn_id.map(str::to_string),
            step_id: step_id.map(str::to_string),
            tool_call_id: tool_call_id.map(str::to_string),
            created_at: Utc::now(),
            state: state.to_string(),
            idempotency_key: idempotency_key.to_string(),
        });
        self.meta.updated_at = Utc::now();
        ConversationEventIdentity {
            id,
            sequence: self.event_sequence,
        }
    }

    pub fn record_tool_call(
        &mut self,
        turn_id: Option<&str>,
        step_id: Option<&str>,
        tool_call_id: &str,
        name: &str,
        arguments: &str,
    ) -> ConversationEventIdentity {
        self.append_event(SessionEvent::ToolCall {
            sequence: 0,
            id: tool_call_id.to_string(),
            turn_id: turn_id.map(str::to_string),
            step_id: step_id.map(str::to_string),
            created_at: Utc::now(),
            name: name.to_string(),
            arguments: arguments.to_string(),
        });
        self.meta.updated_at = Utc::now();
        ConversationEventIdentity {
            id: tool_call_id.to_string(),
            sequence: self.event_sequence,
        }
    }

    pub fn record_tool_result(
        &mut self,
        turn_id: Option<&str>,
        step_id: Option<&str>,
        tool_call_id: &str,
        name: &str,
        content: &str,
        is_error: bool,
        duration_ms: Option<u64>,
    ) -> ConversationEventIdentity {
        self.append_event(SessionEvent::ToolResult {
            sequence: 0,
            id: tool_call_id.to_string(),
            turn_id: turn_id.map(str::to_string),
            step_id: step_id.map(str::to_string),
            created_at: Utc::now(),
            name: name.to_string(),
            content: content.to_string(),
            is_error,
            duration_ms,
        });
        self.meta.updated_at = Utc::now();
        ConversationEventIdentity {
            id: tool_call_id.to_string(),
            sequence: self.event_sequence,
        }
    }

    pub fn record_permission_decision(
        &mut self,
        turn_id: Option<&str>,
        step_id: Option<&str>,
        tool_call_id: &str,
        tool_name: &str,
        allowed: bool,
    ) {
        self.append_event(SessionEvent::PermissionDecision {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            turn_id: turn_id.map(str::to_string),
            step_id: step_id.map(str::to_string),
            tool_call_id: tool_call_id.to_string(),
            created_at: Utc::now(),
            tool_name: tool_name.to_string(),
            allowed,
        });
        self.meta.updated_at = Utc::now();
    }

    pub fn record_mode_changed(&mut self, mode_id: &str) {
        self.append_event(SessionEvent::ModeChanged {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            mode_id: mode_id.to_string(),
        });
        self.meta.updated_at = Utc::now();
    }

    pub fn record_model_changed(&mut self, model: &str) {
        self.append_event(SessionEvent::ModelChanged {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            model: model.to_string(),
        });
        self.meta.updated_at = Utc::now();
    }

    pub fn record_branch_forked(&mut self, source_session_id: &str, branch_session_id: &str) {
        self.append_event(SessionEvent::BranchForked {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            source_session_id: source_session_id.to_string(),
            branch_session_id: branch_session_id.to_string(),
            parent_sequence: Some(self.event_sequence),
        });
        self.meta.updated_at = Utc::now();
    }

    pub fn record_rewound(&mut self, checkpoint_id: &str) {
        self.record_rewound_with_messages(checkpoint_id, Some(self.messages.clone()));
    }

    pub fn record_rewound_with_messages(
        &mut self,
        checkpoint_id: &str,
        messages_after: Option<Vec<Message>>,
    ) {
        self.record_rewound_with_target(checkpoint_id, None, messages_after);
    }

    pub fn record_rewound_with_target(
        &mut self,
        checkpoint_id: &str,
        target_sequence: Option<u64>,
        messages_after: Option<Vec<Message>>,
    ) {
        self.append_event(SessionEvent::Rewound {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            checkpoint_id: checkpoint_id.to_string(),
            target_sequence,
            messages_after,
        });
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
        let message = Message::system(content);
        self.messages.insert(0, message.clone());
        self.append_event(SessionEvent::MessageAppended {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            message,
        });
        self.meta.updated_at = Utc::now();
    }

    /// Replace the leading system prefix through the conversation event log.
    ///
    /// The projected view is authoritative when the materialized cache has
    /// drifted. The cache is refreshed only as a compatibility snapshot after
    /// the event is appended.
    pub fn update_system_prefix(&mut self, content: &str) {
        let view = self.view();
        let mut messages = view.messages;
        if view.fallback_reason == Some(ProjectionFallbackReason::NoEvents) {
            // Seed cache-only legacy sessions before appending the prefix fact;
            // otherwise a later projection would know only about the prefix.
            for message in messages.iter().cloned() {
                self.append_event(SessionEvent::MessageAppended {
                    sequence: 0,
                    id: Uuid::new_v4().to_string(),
                    created_at: Utc::now(),
                    message,
                });
            }
        }
        if let Some(message) = messages.first_mut().filter(|m| m.role == zene_llm::Role::System) {
            if message.content.as_deref() == Some(content) {
                self.messages = messages;
                return;
            }
            message.content = Some(content.to_string());
            self.messages = messages;
            self.append_event(SessionEvent::SystemPrefixChanged {
                sequence: 0,
                id: Uuid::new_v4().to_string(),
                created_at: Utc::now(),
                content: content.to_string(),
            });
        } else {
            let message = Message::system(content);
            messages.insert(0, message.clone());
            self.messages = messages;
            self.append_event(SessionEvent::MessageAppended {
                sequence: 0,
                id: Uuid::new_v4().to_string(),
                created_at: Utc::now(),
                message,
            });
        }
        self.meta.updated_at = Utc::now();
    }

    /// Replace the leading system message, or insert one if missing.
    pub fn set_system_message(&mut self, content: &str) {
        self.update_system_prefix(content);
    }

    pub fn set_title_from_prompt(&mut self, prompt: &str) {
        if self.meta.title == "New session" {
            let title = prompt.lines().next().unwrap_or(prompt);
            self.meta.title = title.chars().take(60).collect();
        }
    }

    /// Replace older messages with a compaction summary, keeping system + recent tail.
    pub fn apply_compaction(&mut self, summary: String, compacted_count: usize) -> CompactionEntry {
        self.record_compaction_event("llm_summarize", compacted_count, Some(summary), None, None)
    }

    pub fn record_compaction_event(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
    ) -> CompactionEntry {
        self.record_compaction_event_with_messages(
            reason,
            compacted_count,
            summary,
            tokens_before,
            tokens_after,
            Some(self.messages.clone()),
        )
    }

    pub fn record_compaction_event_with_messages(
        &mut self,
        reason: &str,
        compacted_count: usize,
        summary: Option<String>,
        tokens_before: Option<u32>,
        tokens_after: Option<u32>,
        messages_after: Option<Vec<Message>>,
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
        self.append_event(SessionEvent::CompactionApplied {
            sequence: 0,
            id: Uuid::new_v4().to_string(),
            created_at: entry.created_at,
            entry: entry.clone(),
            messages_after,
        });
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
        let summary_message =
            Message::compaction_summary(format!("[Previous conversation summary]\n{summary}"));

        self.messages.clear();
        if let Some(system) = system {
            self.messages.push(system);
        }
        self.messages.push(summary_message);
        self.messages.extend(tail);
        self.record_compaction_event_with_messages(
            reason,
            compacted_count,
            Some(summary),
            tokens_before,
            tokens_after,
            Some(self.messages.clone()),
        );
    }

    pub fn save(&self) -> Result<()> {
        FileSessionStore.save(self)
    }

    pub fn save_with_store(&self, store: &dyn SessionStore) -> Result<()> {
        store.save(self)
    }

    pub fn load(id: &str) -> Result<Self> {
        let path = session_path(id);
        let raw = fs::read_to_string(&path)
            .with_context(|| format!("read session file: {}", path.display()))?;
        parse_session_raw(&raw, Some(id)).context("parse session file")
    }

    /// Load a session and explicitly attempt to migrate legacy materialized
    /// records. Safe cache-only records are upgraded in memory; legacy
    /// compaction/rewind or incomplete logs remain legacy for compatibility.
    /// The migrated record is not persisted until the caller invokes `save`.
    pub fn load_migrated(id: &str) -> Result<Self> {
        let mut session = Self::load(id)?;
        session.migrate_to_event_backed();
        Ok(session)
    }

    /// Load and persist a legacy migration at an explicit persistence boundary.
    ///
    /// The store is called only after migration succeeds. If persistence
    /// fails, the error is returned and the loaded record is not returned as
    /// migrated; stores should provide their own atomic-write guarantee when
    /// needed. Retrying is safe because migration is idempotent.
    pub fn load_migrated_with_store(id: &str, store: &dyn SessionStore) -> Result<Self> {
        let mut session = Self::load(id)?;
        if session.migrate_to_event_backed() {
            session
                .save_with_store(store)
                .context("persist migrated session")?;
        }
        Ok(session)
    }

    /// Repair a legacy session using the default file store.
    pub fn repair_legacy(id: &str) -> Result<Self> {
        Self::load_migrated_with_store(id, &FileSessionStore)
    }
}

pub fn session_path(id: &str) -> PathBuf {
    sessions_dir().join(format!("{id}.json"))
}

/// Parse a session JSON document, migrating older shapes that lack `meta`.
pub fn parse_session_raw(raw: &str, fallback_id: Option<&str>) -> Result<SessionRecord> {
    if let Ok(mut record) = serde_json::from_str::<SessionRecord>(raw) {
        record.normalize_event_sequence();
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
        let mut record = legacy_record(
            id.as_deref(),
            title.as_deref(),
            workdir.as_deref(),
            messages,
        );
        if let Some(compactions) = value.get("compactions") {
            if let Ok(parsed) = serde_json::from_value::<Vec<CompactionEntry>>(compactions.clone())
            {
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
            parent_session_id: None,
            parent_sequence: None,
        },
        messages,
        conversation_schema_version: 0,
        events: Vec::new(),
        event_sequence: 0,
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
                eprintln!("skipping unreadable session file {}: {err}", path.display());
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

pub use record::{
    export_session, record_path, session_record_dir, AgentRecordWriter, ExecutionCheckpointState,
    RecordEntry, RecoveryDisposition, RecoveryExecution, RecoveryPlan, RecoverySnapshot,
    ResumeCandidate,
};

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
        assert_eq!(
            session.messages[0].content.as_deref(),
            Some("system prompt")
        );
    }

    #[test]
    fn push_message_skips_duplicate_system() {
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("system prompt");
        session.push_message(Message::system("another system"));
        assert_eq!(session.messages.len(), 1);
    }

    #[test]
    fn system_prefix_update_is_event_backed_when_cache_drifts() {
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("original");
        session.push_message(Message::user("request"));
        session.messages[0].content = Some("stale cache".into());

        session.update_system_prefix("updated");

        let view = session.try_view().expect("system prefix event is projectable");
        assert_eq!(view.messages[0].content.as_deref(), Some("updated"));
        assert_eq!(view.messages[1].content.as_deref(), Some("request"));
        assert_eq!(session.messages[0].content.as_deref(), Some("updated"));
        assert!(matches!(
            session.events.last(),
            Some(SessionEvent::SystemPrefixChanged { content, .. }) if content == "updated"
        ));
    }

    #[test]
    fn compaction_replaces_prefix_and_records_history() {
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("system prompt");
        session.push_message(Message::user("old request"));
        session.push_message(Message::assistant("old reply"));
        session.push_message(Message::user("recent request"));
        session.push_message(Message::assistant("recent reply"));

        session.replace_messages_after_compaction("summarized old work".into(), 3, 2);

        assert_eq!(session.messages.len(), 4);
        assert_eq!(
            session.messages[0].content.as_deref(),
            Some("system prompt")
        );
        assert_eq!(
            session.messages[1].kind,
            Some(zene_llm::MessageKind::CompactionSummary)
        );
        assert!(session.messages[1]
            .content
            .as_deref()
            .unwrap()
            .contains("summarized old work"));
        assert_eq!(
            session.messages[2].content.as_deref(),
            Some("recent request")
        );
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
        assert_eq!(loaded.events.len(), 2);
        assert!(matches!(
            loaded.events[0],
            SessionEvent::MessageAppended { .. }
        ));
        assert!(matches!(
            loaded.events[1],
            SessionEvent::CompactionApplied { .. }
        ));
    }

    #[test]
    fn session_view_rebuilds_messages_and_compaction_snapshot() {
        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(Message::system("sys"));
        session.push_message(Message::user("before"));
        session.replace_messages_after_compaction("summary".into(), 1, 1);
        session.push_message(Message::assistant("after"));

        let view = session.view();
        assert!(!view.used_materialized_fallback);
        assert_eq!(view.source_event_count, session.events.len());
        assert_eq!(view.active_events.len(), session.events.len());
        assert_eq!(
            serde_json::to_string(&view.messages).unwrap(),
            serde_json::to_string(&session.messages).unwrap(),
        );
    }

    #[test]
    fn event_backed_compaction_projection_does_not_need_materialized_cache() {
        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(Message::system("sys"));
        session.push_message(Message::user("before"));
        session.replace_messages_after_compaction("summary".into(), 1, 1);
        session.push_message(Message::assistant("after"));
        let expected = session.view().messages;
        session.messages.clear();

        let strict = session
            .try_view()
            .expect("compaction snapshot is event-backed");
        assert_eq!(
            serde_json::to_vec(&strict.messages).unwrap(),
            serde_json::to_vec(&expected).unwrap()
        );
        assert!(!strict.used_materialized_fallback);
    }

    #[test]
    fn compaction_projection_survives_session_reload() {
        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(Message::system("sys"));
        session.push_message(Message::user("before"));
        session.push_message(Message::assistant("old reply"));
        session.replace_messages_after_compaction("summary".into(), 1, 2);
        session.push_message(Message::user("after reload"));

        let before = session.view();
        let raw = serde_json::to_string(&session).expect("serialize session");
        let reloaded: SessionRecord = serde_json::from_str(&raw).expect("reload session");
        let after = reloaded.view();

        assert_eq!(
            serde_json::to_vec(&before.messages).unwrap(),
            serde_json::to_vec(&after.messages).unwrap()
        );
        assert_eq!(before.source_event_count, after.source_event_count);
        assert_eq!(before.active_events.len(), after.active_events.len());
        assert_eq!(before.fallback_reason, after.fallback_reason);
        assert!(!after.used_materialized_fallback);
    }

    #[test]
    fn fork_projection_uses_parent_and_branch_local_events() {
        let mut source = SessionRecord::new(Path::new("."));
        source.push_message(Message::user("parent"));
        let mut fork = crate::checkpoint::fork_session(&source, Path::new("."));
        assert_eq!(
            fork.meta.parent_session_id.as_deref(),
            Some(source.meta.id.as_str())
        );
        assert_eq!(fork.meta.parent_sequence, Some(source.event_sequence));
        fork.push_message(Message::assistant("branch"));

        let view = fork.view();
        assert_eq!(
            view.active_branch_id.as_deref(),
            Some(fork.meta.id.as_str())
        );
        assert_eq!(view.active_path_start_sequence, Some(1));
        assert_eq!(view.active_events.len(), 3);
        let projected = serde_json::to_string(&view.messages).unwrap();
        assert!(projected.contains("parent"));
        assert!(projected.contains("branch"));
        assert_eq!(view.source_event_count, fork.events.len());
    }

    #[test]
    fn nested_fork_projection_keeps_each_branch_local_suffix() {
        let mut root = SessionRecord::new(Path::new("."));
        root.push_message(Message::user("root"));
        let mut first = crate::checkpoint::fork_session(&root, Path::new("."));
        first.push_message(Message::assistant("first branch"));
        let first_id = first.meta.id.clone();

        let mut second = crate::checkpoint::fork_session(&first, Path::new("."));
        second.push_message(Message::assistant("second branch"));
        let view = second.view();
        let projected = serde_json::to_string(&view.messages).unwrap();

        assert_eq!(
            second.meta.parent_session_id.as_deref(),
            Some(first_id.as_str())
        );
        assert!(projected.contains("root"));
        assert!(projected.contains("first branch"));
        assert!(projected.contains("second branch"));
        assert!(!projected.contains("sibling branch"));
        assert_eq!(
            view.active_branch_id.as_deref(),
            Some(second.meta.id.as_str())
        );
    }

    #[test]
    fn sibling_fork_projection_does_not_leak_other_branch_messages() {
        let mut root = SessionRecord::new(Path::new("."));
        root.push_message(Message::user("root"));
        let mut left = crate::checkpoint::fork_session(&root, Path::new("."));
        left.push_message(Message::assistant("left only"));
        let mut right = crate::checkpoint::fork_session(&root, Path::new("."));
        right.push_message(Message::assistant("right only"));

        let left_view = left.view();
        let right_view = right.view();
        let left_json = serde_json::to_string(&left_view.messages).unwrap();
        let right_json = serde_json::to_string(&right_view.messages).unwrap();
        assert!(left_json.contains("left only"));
        assert!(!left_json.contains("right only"));
        assert!(right_json.contains("right only"));
        assert!(!right_json.contains("left only"));
    }

    #[test]
    fn rewind_projection_keeps_prior_facts_and_resets_active_view() {
        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(Message::user("before"));
        let checkpoint_messages = session.messages.clone();
        let checkpoint_sequence = session.event_sequence;
        session.push_message(Message::assistant("later"));
        session.messages = checkpoint_messages.clone();
        session.record_rewound_with_target(
            "cp-1",
            Some(checkpoint_sequence),
            Some(checkpoint_messages.clone()),
        );
        session.push_message(Message::user("after rewind"));
        let view = session.view();
        assert_eq!(
            serde_json::to_string(&view.messages).unwrap(),
            serde_json::to_string(&[Message::user("before"), Message::user("after rewind"),])
                .unwrap(),
            "active events: {:?}, fallback: {:?}",
            view.active_events,
            view.fallback_reason,
        );
        assert_eq!(view.active_events.len(), 3);
        assert_eq!(view.active_events[0].sequence(), 1);
        assert!(view.active_events.iter().all(|event| {
            !matches!(event, SessionEvent::MessageAppended { message, .. } if message.content.as_deref() == Some("later"))
        }));
        assert!(!view.used_materialized_fallback);
        assert!(session.events.len() >= 4);
        assert!(matches!(
            session.events.last(),
            Some(SessionEvent::MessageAppended { .. })
        ));
    }

    #[test]
    fn legacy_projection_fallback_exposes_reason() {
        let mut session = SessionRecord::new(Path::new("."));
        session.messages = vec![Message::user("cached")];
        session.events.push(SessionEvent::CompactionApplied {
            sequence: 1,
            id: "legacy-compact".into(),
            created_at: Utc::now(),
            entry: CompactionEntry {
                id: "compact".into(),
                created_at: Utc::now(),
                summary: "old".into(),
                compacted_message_count: 1,
                reason: None,
                tokens_before: None,
                tokens_after: None,
            },
            messages_after: None,
        });
        let view = session.view();
        assert!(view.used_materialized_fallback);
        assert_eq!(
            view.fallback_reason,
            Some(ProjectionFallbackReason::LegacyCompactionWithoutSnapshot)
        );
        assert_eq!(
            serde_json::to_vec(&view.messages).unwrap(),
            serde_json::to_vec(&session.messages).unwrap()
        );
    }

    #[test]
    fn conversation_event_identity_tracks_id_and_sequence() {
        let mut session = SessionRecord::new(Path::new("."));
        let first = session.record_turn_started("turn-1", "hello");
        let second = session.record_checkpoint(
            Some("turn-1"),
            None,
            None,
            "turn_started",
            "turn-1/started",
        );
        assert_ne!(first.id, second.id);
        assert!(first.sequence < second.sequence);
        assert_eq!(session.events()[0].sequence(), first.sequence);
        assert_eq!(session.events()[1].sequence(), second.sequence);
    }

    #[test]
    fn event_projection_prefers_facts_when_materialized_cache_drifts() {
        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(Message::user("event fact"));
        session.messages.push(Message::assistant("cache drift"));
        let view = session.view();
        assert!(!view.used_materialized_fallback);
        assert!(view.cache_drift_detected);
        assert_eq!(view.fallback_reason, None);
        assert_eq!(
            serde_json::to_vec(&view.messages).unwrap(),
            serde_json::to_vec(&[Message::user("event fact")]).unwrap()
        );
    }

    #[test]
    fn empty_new_session_has_no_projection_fallback() {
        let session = SessionRecord::new(Path::new("."));
        assert!(session.is_event_backed());
        let view = session.view();
        assert!(!view.used_materialized_fallback);
        assert!(!view.cache_drift_detected);
        assert_eq!(view.fallback_reason, None);
    }

    #[test]
    fn event_backed_projection_survives_empty_materialized_cache() {
        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(Message::system("sys"));
        session.push_message(Message::user("hello"));
        let expected = session.view().messages;
        session.messages.clear();

        let view = session.view();
        assert_eq!(
            serde_json::to_vec(&view.messages).unwrap(),
            serde_json::to_vec(&expected).unwrap(),
        );
        assert!(!view.used_materialized_fallback);
        assert!(view.cache_drift_detected);
    }

    #[test]
    fn legacy_cache_only_session_migrates_idempotently() {
        let mut session = legacy_record(
            Some("legacy"),
            None,
            None,
            vec![Message::user("hello"), Message::assistant("reply")],
        );
        assert!(!session.is_event_backed());
        assert!(session.migrate_to_event_backed());
        assert!(!session.migrate_to_event_backed());
        assert!(session.is_event_backed());
        assert_eq!(session.events.len(), 2);
        session.messages.clear();
        let view = session.view();
        assert_eq!(view.messages.len(), 2);
        assert!(!view.used_materialized_fallback);
        assert_eq!(view.fallback_reason, None);
    }

    #[test]
    fn legacy_compaction_does_not_migrate_without_snapshot() {
        let mut session = legacy_record(
            Some("legacy-compaction"),
            None,
            None,
            vec![Message::user("cached")],
        );
        session.events.push(SessionEvent::CompactionApplied {
            sequence: 1,
            id: "compact-1".into(),
            created_at: Utc::now(),
            entry: CompactionEntry {
                id: "entry-1".into(),
                created_at: Utc::now(),
                summary: "legacy summary".into(),
                compacted_message_count: 1,
                reason: None,
                tokens_before: None,
                tokens_after: None,
            },
            messages_after: None,
        });
        assert!(!session.migrate_to_event_backed());
        assert!(!session.is_event_backed());
        assert!(!session.migrate_to_event_backed());
        assert!(session.view().used_materialized_fallback);
        assert_eq!(
            session.try_view().unwrap_err(),
            ProjectionFallbackReason::LegacyCompactionWithoutSnapshot
        );
    }

    #[test]
    fn legacy_rewind_does_not_migrate_without_snapshot() {
        let mut session = legacy_record(
            Some("legacy-rewind"),
            None,
            None,
            vec![Message::user("cached")],
        );
        session.events.push(SessionEvent::Rewound {
            sequence: 1,
            id: "rewind-1".into(),
            created_at: Utc::now(),
            checkpoint_id: "checkpoint-1".into(),
            target_sequence: None,
            messages_after: None,
        });
        assert!(!session.migrate_to_event_backed());
        assert!(!session.is_event_backed());
        assert!(session.view().used_materialized_fallback);
        assert_eq!(
            session.try_view().unwrap_err(),
            ProjectionFallbackReason::LegacyRewindWithoutSnapshot
        );
    }

    #[test]
    fn incomplete_legacy_event_log_does_not_migrate() {
        let mut session = legacy_record(
            Some("legacy-incomplete"),
            None,
            None,
            vec![Message::user("cached")],
        );
        session.events.push(SessionEvent::TurnStarted {
            sequence: 1,
            id: "turn-1".into(),
            turn_id: "turn-1".into(),
            created_at: Utc::now(),
            prompt: "cached".into(),
        });
        assert!(!session.migrate_to_event_backed());
        assert!(!session.is_event_backed());
        assert_eq!(
            session.view().fallback_reason,
            Some(ProjectionFallbackReason::IncompleteEventLog)
        );
    }

    #[test]
    fn load_migrated_with_store_persists_once_and_is_idempotent() {
        let _guard = ZENE_HOME_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ZENE_HOME", dir.path());
        fs::create_dir_all(sessions_dir()).unwrap();

        let legacy = legacy_record(
            Some("repair-once"),
            None,
            None,
            vec![Message::user("hello")],
        );
        fs::write(
            session_path("repair-once"),
            serde_json::to_string(&legacy).unwrap(),
        )
        .unwrap();

        struct CountingStore(std::sync::Mutex<usize>);
        impl SessionStore for CountingStore {
            fn save(&self, session: &SessionRecord) -> Result<()> {
                *self.0.lock().unwrap() += 1;
                FileSessionStore.save(session)
            }
        }

        let store = CountingStore(std::sync::Mutex::new(0));
        let first = SessionRecord::load_migrated_with_store("repair-once", &store).unwrap();
        assert!(first.is_event_backed());
        let second = SessionRecord::load_migrated_with_store("repair-once", &store).unwrap();
        assert!(second.is_event_backed());
        assert_eq!(*store.0.lock().unwrap(), 1);
        assert_eq!(SessionRecord::load("repair-once").unwrap().events.len(), 1);

        std::env::remove_var("ZENE_HOME");
    }

    #[test]
    fn failed_migration_persistence_leaves_legacy_file_unchanged() {
        let _guard = ZENE_HOME_TEST_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        std::env::set_var("ZENE_HOME", dir.path());
        fs::create_dir_all(sessions_dir()).unwrap();

        let legacy = legacy_record(
            Some("repair-fails"),
            None,
            None,
            vec![Message::user("hello")],
        );
        let raw = serde_json::to_string(&legacy).unwrap();
        fs::write(session_path("repair-fails"), &raw).unwrap();

        struct FailingStore;
        impl SessionStore for FailingStore {
            fn save(&self, _session: &SessionRecord) -> Result<()> {
                Err(anyhow::anyhow!("injected persistence failure"))
            }
        }

        let error = SessionRecord::load_migrated_with_store("repair-fails", &FailingStore)
            .expect_err("migration persistence should fail");
        assert!(error.to_string().contains("persist migrated session"));
        assert_eq!(
            fs::read_to_string(session_path("repair-fails")).unwrap(),
            raw
        );
        assert!(!SessionRecord::load("repair-fails")
            .unwrap()
            .is_event_backed());

        std::env::remove_var("ZENE_HOME");
    }

    #[test]
    fn session_messages_dual_write_conversation_events() {
        let mut session = SessionRecord::new(Path::new("."));
        session.ensure_system_message("sys");
        session.push_message(Message::user("hello"));

        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.events.len(), 2);
        assert!(matches!(
            session.events[0],
            SessionEvent::MessageAppended { .. }
        ));
        assert!(matches!(
            session.events[1],
            SessionEvent::MessageAppended { .. }
        ));
    }

    #[test]
    fn conversation_events_are_scoped_and_monotonic() {
        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(Message::user("hello"));
        session.record_tool_call(Some("turn-1"), Some("step-1"), "call-1", "Read", "{}");
        session.record_permission_decision(Some("turn-1"), Some("step-1"), "call-1", "Read", true);
        session.record_tool_result(
            Some("turn-1"),
            Some("step-1"),
            "call-1",
            "Read",
            "contents",
            false,
            Some(12),
        );
        session.record_mode_changed("plan");
        session.record_model_changed("test-model");

        assert_eq!(session.event_sequence, 6);
        let sequences: Vec<u64> = session.events.iter().map(SessionEvent::sequence).collect();
        assert_eq!(sequences, vec![1, 2, 3, 4, 5, 6]);
        assert!(
            matches!(session.events[1], SessionEvent::ToolCall { ref turn_id, ref step_id, .. } if turn_id.as_deref() == Some("turn-1") && step_id.as_deref() == Some("step-1"))
        );
        assert!(matches!(
            session.events[2],
            SessionEvent::PermissionDecision { allowed: true, .. }
        ));
        assert!(matches!(
            session.events[3],
            SessionEvent::ToolResult {
                duration_ms: Some(12),
                ..
            }
        ));
        assert!(
            matches!(session.events[4], SessionEvent::ModeChanged { ref mode_id, .. } if mode_id == "plan")
        );
        assert!(
            matches!(session.events[5], SessionEvent::ModelChanged { ref model, .. } if model == "test-model")
        );
    }

    #[test]
    fn newer_snapshot_recovers_after_legacy_compaction_event() {
        let mut session = SessionRecord::new(Path::new("."));
        session.push_message(Message::user("before"));
        session.events.push(SessionEvent::CompactionApplied {
            sequence: 99,
            id: "legacy-compact".into(),
            created_at: Utc::now(),
            entry: CompactionEntry {
                id: "compact".into(),
                created_at: Utc::now(),
                summary: "old".into(),
                compacted_message_count: 1,
                reason: None,
                tokens_before: None,
                tokens_after: None,
            },
            messages_after: None,
        });
        session.replace_messages_after_compaction("new".into(), 0, 1);
        let view = session.view();
        assert!(!view.used_materialized_fallback);
    }

    #[test]
    fn legacy_event_sequences_are_rebuilt_on_parse() {
        let raw = r#"{
            "meta": {
                "id": "legacy-sequences",
                "title": "Legacy",
                "workdir": ".",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            },
            "messages": [{"role":"user","content":"hello"}],
            "events": [{"type":"message_appended","id":"m1","created_at":"2026-01-01T00:00:00Z","message":{"role":"user","content":"hello"}}]
        }"#;
        let mut session = parse_session_raw(raw, None).expect("parse legacy event");
        assert_eq!(session.event_sequence, 1);
        assert!(matches!(
            session.events[0],
            SessionEvent::MessageAppended { sequence: 1, .. }
        ));
        session.push_message(Message::assistant("reply"));
        assert!(matches!(
            session.events[1],
            SessionEvent::MessageAppended { sequence: 2, .. }
        ));
    }

    #[test]
    fn session_store_receives_complete_snapshot() {
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct RecordingStore(Arc<Mutex<Vec<String>>>);

        impl SessionStore for RecordingStore {
            fn save(&self, session: &SessionRecord) -> Result<()> {
                self.0.lock().unwrap().push(session.meta.id.clone());
                Ok(())
            }
        }

        let store = RecordingStore::default();
        let mut session = SessionRecord::new(Path::new("."));
        let id = session.meta.id.clone();
        session.push_message(Message::user("persist me"));
        session.save_with_store(&store).expect("store snapshot");
        assert_eq!(store.0.lock().unwrap().as_slice(), [id]);
    }

    #[test]
    fn old_session_without_events_remains_compatible() {
        let raw = r#"{
            "meta": {
                "id": "legacy-events",
                "title": "Legacy",
                "workdir": ".",
                "created_at": "2026-01-01T00:00:00Z",
                "updated_at": "2026-01-01T00:00:00Z"
            },
            "messages": []
        }"#;
        let loaded: SessionRecord = serde_json::from_str(raw).expect("deserialize legacy");
        assert!(loaded.events.is_empty());
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
