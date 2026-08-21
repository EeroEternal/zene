use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type Id = Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct User {
    pub id: Id,
    pub email: String,
    pub display_name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    pub id: Id,
    pub slug: String,
    pub name: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    pub id: Id,
    pub organization_id: Id,
    pub provider: String,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub clone_url: String,
    pub installation_id: Option<String>,
    pub provider_repo_id: Option<String>,
    pub private: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Created,
    Queued,
    Provisioning,
    Cloning,
    Starting,
    Running,
    WaitingForApproval,
    WaitingForUser,
    Stopping,
    Completed,
    Failed,
    TimedOut,
    Cancelled,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Created => "created",
            Self::Queued => "queued",
            Self::Provisioning => "provisioning",
            Self::Cloning => "cloning",
            Self::Starting => "starting",
            Self::Running => "running",
            Self::WaitingForApproval => "waiting_for_approval",
            Self::WaitingForUser => "waiting_for_user",
            Self::Stopping => "stopping",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::TimedOut => "timed_out",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "created" => Self::Created,
            "queued" => Self::Queued,
            "provisioning" => Self::Provisioning,
            "cloning" => Self::Cloning,
            "starting" => Self::Starting,
            "running" => Self::Running,
            "waiting_for_approval" => Self::WaitingForApproval,
            "waiting_for_user" => Self::WaitingForUser,
            "stopping" => Self::Stopping,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "timed_out" => Self::TimedOut,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }

    pub fn accepts_messages(self) -> bool {
        matches!(
            self,
            Self::Queued
                | Self::Provisioning
                | Self::Cloning
                | Self::Starting
                | Self::Running
                | Self::WaitingForApproval
                | Self::WaitingForUser
                | Self::Completed
        )
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Failed | Self::TimedOut | Self::Cancelled)
    }
}

/// Stored run permission mode. Matches Console `PermissionMode`.
/// `auto` is a historical alias; it still auto-resolves approvals.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    #[default]
    Default,
    AcceptEdits,
    Yolo,
    Auto,
}

impl PermissionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::AcceptEdits => "accept_edits",
            Self::Yolo => "yolo",
            Self::Auto => "auto",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "default" => Self::Default,
            "accept_edits" => Self::AcceptEdits,
            "yolo" => Self::Yolo,
            "auto" => Self::Auto,
            _ => return None,
        })
    }

    /// `default` / `yolo` / `auto` auto-resolve approval rows on create.
    pub fn auto_resolves_approvals(self) -> bool {
        matches!(self, Self::Default | Self::Yolo | Self::Auto)
    }
}

/// Stored chat turn author. Matches Console bubble roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
}

impl MessageRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: Id,
    pub organization_id: Id,
    pub repository_id: Id,
    /// Persistent checkout shared by sessions for the same org+repo.
    pub workspace_id: Id,
    pub requested_by: Id,
    pub status: RunStatus,
    pub status_version: i64,
    pub title: String,
    pub prompt: String,
    pub base_ref: String,
    pub base_sha: Option<String>,
    pub head_branch: String,
    pub head_sha: Option<String>,
    pub model: String,
    pub permission_mode: PermissionMode,
    /// Agent step budget for this run; `0` means unlimited.
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRunRequest {
    pub title: Option<String>,
    pub archived: Option<bool>,
}

/// Product `event_type` written by RuntimeClient for classified frames.
/// Unrecognized ACP frames are `acp` (with a residual product payload when the
/// frame is a `session/update`). Platform / legacy `runtime` rows use
/// [`RunEventKind`]; `RunEvent.event_type` stays a string so unknown historical
/// values still load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudEventKind {
    TextDelta,
    ThoughtDelta,
    UserMessage,
    ToolCall,
    ToolResult,
    StateChanged,
    UsageUpdate,
    ProjectionReady,
    Plan,
    AvailableCommands,
    SessionStarted,
    ApprovalRequested,
    /// ACP `initialize` handshake result (not a session).
    Initialized,
    /// Reverse request that Cloud auto-rejects (no permission mapping).
    UnsupportedRequest,
    TurnStarted,
    StepStarted,
    TurnEnded,
    Error,
    Acp,
}

impl CloudEventKind {
    pub fn as_event_type(self) -> &'static str {
        match self {
            Self::TextDelta => "text_delta",
            Self::ThoughtDelta => "thought_delta",
            Self::UserMessage => "user_message",
            Self::ToolCall => "tool_call",
            Self::ToolResult => "tool_result",
            Self::StateChanged => "state_changed",
            Self::UsageUpdate => "usage_update",
            Self::ProjectionReady => "projection_ready",
            Self::Plan => "plan",
            Self::AvailableCommands => "available_commands",
            Self::SessionStarted => "session_started",
            Self::ApprovalRequested => "approval_requested",
            Self::Initialized => "initialized",
            Self::UnsupportedRequest => "unsupported_request",
            Self::TurnStarted => "turn_started",
            Self::StepStarted => "step_started",
            Self::TurnEnded => "turn_ended",
            Self::Error => "error",
            Self::Acp => "acp",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "text_delta" => Self::TextDelta,
            "thought_delta" => Self::ThoughtDelta,
            "user_message" => Self::UserMessage,
            "tool_call" => Self::ToolCall,
            "tool_result" => Self::ToolResult,
            "state_changed" => Self::StateChanged,
            "usage_update" => Self::UsageUpdate,
            "projection_ready" => Self::ProjectionReady,
            "plan" => Self::Plan,
            "available_commands" => Self::AvailableCommands,
            "session_started" => Self::SessionStarted,
            "approval_requested" => Self::ApprovalRequested,
            "initialized" => Self::Initialized,
            "unsupported_request" => Self::UnsupportedRequest,
            "turn_started" => Self::TurnStarted,
            "step_started" => Self::StepStarted,
            "turn_ended" => Self::TurnEnded,
            "error" => Self::Error,
            "acp" => Self::Acp,
            _ => return None,
        })
    }
}

/// Stored `event_type` written by Cloud (runtime client, platform DB rows, and
/// worker/API). Wire JSON stays snake_case strings. `RunEvent.event_type` remains
/// a string on read so unknown historical values still load.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunEventKind {
    TextDelta,
    ThoughtDelta,
    UserMessage,
    ToolCall,
    ToolResult,
    StateChanged,
    UsageUpdate,
    ProjectionReady,
    Plan,
    AvailableCommands,
    SessionStarted,
    ApprovalRequested,
    Initialized,
    UnsupportedRequest,
    TurnStarted,
    StepStarted,
    TurnEnded,
    Error,
    Acp,
    Platform,
    Runtime,
}

impl From<CloudEventKind> for RunEventKind {
    fn from(kind: CloudEventKind) -> Self {
        match kind {
            CloudEventKind::TextDelta => Self::TextDelta,
            CloudEventKind::ThoughtDelta => Self::ThoughtDelta,
            CloudEventKind::UserMessage => Self::UserMessage,
            CloudEventKind::ToolCall => Self::ToolCall,
            CloudEventKind::ToolResult => Self::ToolResult,
            CloudEventKind::StateChanged => Self::StateChanged,
            CloudEventKind::UsageUpdate => Self::UsageUpdate,
            CloudEventKind::ProjectionReady => Self::ProjectionReady,
            CloudEventKind::Plan => Self::Plan,
            CloudEventKind::AvailableCommands => Self::AvailableCommands,
            CloudEventKind::SessionStarted => Self::SessionStarted,
            CloudEventKind::ApprovalRequested => Self::ApprovalRequested,
            CloudEventKind::Initialized => Self::Initialized,
            CloudEventKind::UnsupportedRequest => Self::UnsupportedRequest,
            CloudEventKind::TurnStarted => Self::TurnStarted,
            CloudEventKind::StepStarted => Self::StepStarted,
            CloudEventKind::TurnEnded => Self::TurnEnded,
            CloudEventKind::Error => Self::Error,
            CloudEventKind::Acp => Self::Acp,
        }
    }
}

impl RunEventKind {
    pub fn as_event_type(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::Runtime => "runtime",
            Self::TextDelta => CloudEventKind::TextDelta.as_event_type(),
            Self::ThoughtDelta => CloudEventKind::ThoughtDelta.as_event_type(),
            Self::UserMessage => CloudEventKind::UserMessage.as_event_type(),
            Self::ToolCall => CloudEventKind::ToolCall.as_event_type(),
            Self::ToolResult => CloudEventKind::ToolResult.as_event_type(),
            Self::StateChanged => CloudEventKind::StateChanged.as_event_type(),
            Self::UsageUpdate => CloudEventKind::UsageUpdate.as_event_type(),
            Self::ProjectionReady => CloudEventKind::ProjectionReady.as_event_type(),
            Self::Plan => CloudEventKind::Plan.as_event_type(),
            Self::AvailableCommands => CloudEventKind::AvailableCommands.as_event_type(),
            Self::SessionStarted => CloudEventKind::SessionStarted.as_event_type(),
            Self::ApprovalRequested => CloudEventKind::ApprovalRequested.as_event_type(),
            Self::Initialized => CloudEventKind::Initialized.as_event_type(),
            Self::UnsupportedRequest => CloudEventKind::UnsupportedRequest.as_event_type(),
            Self::TurnStarted => CloudEventKind::TurnStarted.as_event_type(),
            Self::StepStarted => CloudEventKind::StepStarted.as_event_type(),
            Self::TurnEnded => CloudEventKind::TurnEnded.as_event_type(),
            Self::Error => CloudEventKind::Error.as_event_type(),
            Self::Acp => CloudEventKind::Acp.as_event_type(),
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "platform" => Some(Self::Platform),
            "runtime" => Some(Self::Runtime),
            other => CloudEventKind::parse(other).map(Self::from),
        }
    }
}

/// Classified product payloads persisted on `RunEvent.payload`.
/// Wire JSON stays camelCase; ACP envelopes stay inside the adapter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TextEventPayload {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolCallPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_output: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionStartedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_mode_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_modes: Option<serde_json::Value>,
    /// Inspect-only recovery snapshot from ACP `_meta.recovery`.
    /// Never used to auto-resume or auto-replay pending tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<SessionRecoveryPayload>,
}

/// Product recovery fields lifted from ACP session new/load/resume `_meta`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecoveryPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disposition: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_incomplete_execution: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_turn_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_tool_count: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub safe_resume_allowed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automatic_resume: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Product payload for ACP `initialize` results.
/// Extra result keys are preserved so the write path does not drop fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct InitializedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_capabilities: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_info: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_methods: Option<serde_json::Value>,
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Product payload for reverse requests Cloud auto-rejects.
/// JSON-RPC `id` stays inside the adapter and is never persisted here.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnsupportedRequestPayload {
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// Residual product payload for unknown `session/update` frames.
/// Keeps method / sessionUpdate / update body without the JSON-RPC envelope.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AcpResidualPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub method: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_update: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub update: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TurnStartedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StepStartedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct TurnEndedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ErrorPayload {
    #[serde(default)]
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct StateChangedPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsagePayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_tokens: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completion_tokens: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_percent: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_epoch: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cached_tokens: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct PlanPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entries: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct AvailableCommandsPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_commands: Option<serde_json::Value>,
}

/// Product payload stored on `projection_ready` rows.
/// Extra `_meta` keys are preserved so the write path does not drop fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectionPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_message_count: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projected_message_count: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_epoch: Option<serde_json::Value>,
    #[serde(default, flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalEventPayload {
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_input: Option<serde_json::Value>,
}

impl ApprovalEventPayload {
    pub fn is_ask_user(&self) -> bool {
        self.raw_input.as_ref().is_some_and(|value| {
            value.get("askUser").and_then(|flag| flag.as_bool()) == Some(true)
        })
    }
}

/// Product payload stored on `event_type = "platform"` rows.
/// Wire JSON keeps the `event` discriminator and camelCase fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all_fields = "camelCase")]
pub enum PlatformEvent {
    #[serde(rename = "run.created")]
    RunCreated { title: String, prompt: String },
    #[serde(rename = "run.title")]
    RunTitle { title: String },
    #[serde(rename = "run.archived")]
    RunArchived,
    #[serde(rename = "run.status")]
    RunStatusChanged {
        status: RunStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        head_sha: Option<String>,
    },
    #[serde(rename = "message.created")]
    MessageCreated { role: MessageRole, text: String },
    #[serde(rename = "approval.created")]
    ApprovalCreated {
        approval_id: Id,
        status: ApprovalStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        decision: Option<ApprovalDecision>,
        kind: ApprovalKind,
    },
    #[serde(rename = "approval.decided")]
    ApprovalDecided {
        approval_id: Id,
        decision: ApprovalDecision,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: Id,
    pub seq: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<u64>,
    pub event_type: String,
    pub payload: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunMessage {
    pub id: Id,
    pub run_id: Id,
    pub author_id: Option<Id>,
    pub role: MessageRole,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRunRequest {
    pub repository_id: Id,
    pub prompt: String,
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default)]
    pub permission_mode: PermissionMode,
    /// Agent step budget; `0` = unlimited. Defaults to 100.
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
    /// Optional ACP session mode (`plan` / `default` / …). Applied via
    /// `RuntimeCommand::SetMode` when the worker is idle. Orthogonal to
    /// [`PermissionMode`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<String>,
}

fn default_branch_name() -> String {
    "main".into()
}

fn default_model() -> String {
    "default".into()
}

fn default_max_turns() -> u32 {
    100
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostMessageRequest {
    pub text: String,
    pub client_message_id: Option<String>,
}

/// Request to queue an ACP session mode change for the worker.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRunModeRequest {
    pub mode_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub display_name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailLoginRequest {
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailLoginResponse {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub login_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthResponse {
    pub token: String,
    pub user: User,
    pub organization: Organization,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRepositoryRequest {
    pub owner: String,
    pub name: String,
    #[serde(default = "default_branch_name")]
    pub default_branch: String,
    pub clone_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiError {
    pub error: String,
    pub message: String,
    pub retryable: bool,
    pub trace_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClaimedRun {
    pub run: Run,
    pub attempt_id: Id,
    pub generation: i64,
    /// Existing runtime session to resume after a worker replacement, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_session_id: Option<String>,
    #[serde(default)]
    pub resume_without_prompt: bool,
    pub workspace_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerClaimRequest {
    pub worker_id: String,
    pub workspace_root: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerFence {
    pub attempt_id: Id,
    pub generation: i64,
    pub worker_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueHold {
    pub worker_id: String,
    pub run_id: Id,
    pub since: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueActive {
    pub worker_id: String,
    pub run_id: Id,
    pub status: RunStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueueStats {
    pub queued: u64,
    pub active: u64,
    pub holding: u64,
    pub holds: Vec<QueueHold>,
    pub actives: Vec<QueueActive>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerEventRequest {
    pub source_event_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<u64>,
    pub event_type: RunEventKind,
    pub payload: serde_json::Value,
    #[serde(flatten)]
    pub fence: Option<WorkerFence>,
}

#[cfg(test)]
mod tests {
    use super::{
        github_account_view, ApprovalDecision, ApprovalEventPayload, ApprovalKind, ApprovalRisk,
        ApprovalStatus, CloudEventKind, CreateApprovalRequest, DecideApprovalRequest, GithubAccount,
        GithubAccountType, GithubInstallationStatus,
        GithubMode, MessageRole, PermissionMode, PlatformEvent, ProjectionPayload,
        PullRequestState, RunEventKind, RunStatus, TextEventPayload, ToolCallPayload,
        WorkerClaimRequest, WorkerCommand, WorkerCommandAckRequest, WorkerCommandKind,
        WorkerEventRequest, WorkerFence, WorkerPushRequest, WorkerSessionRequest,
    };

    #[test]
    fn github_account_view_omits_access_token() {
        let view = github_account_view(&GithubAccount {
            id: uuid::Uuid::nil(),
            user_id: uuid::Uuid::nil(),
            github_user_id: "1".into(),
            login: "octocat".into(),
            access_token_enc: "gho_secret".into(),
            token_type: "bearer".into(),
            scope: None,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        });
        assert_eq!(view["login"], "octocat");
        assert!(view.get("accessTokenEnc").is_none());
        assert!(view.get("access_token_enc").is_none());
    }

    #[test]
    fn worker_command_ack_round_trips_fence() {
        let request = WorkerCommandAckRequest {
            message_id: uuid::Uuid::nil(),
            fence: WorkerFence {
                attempt_id: uuid::Uuid::nil(),
                generation: 2,
                worker_id: "worker-1".into(),
            },
        };
        let encoded = serde_json::to_value(request).expect("ack should serialize");
        assert_eq!(encoded["messageId"], uuid::Uuid::nil().to_string());
        assert_eq!(encoded["generation"], 2);
    }

    #[test]
    fn worker_claim_and_session_requests_round_trip_camel_case() {
        let encoded = serde_json::to_value(WorkerClaimRequest {
            worker_id: "worker-1".into(),
            workspace_root: "/tmp/ws".into(),
        })
        .expect("claim");
        assert_eq!(
            encoded,
            serde_json::json!({
                "workerId": "worker-1",
                "workspaceRoot": "/tmp/ws"
            })
        );
        let encoded = serde_json::to_value(WorkerSessionRequest {
            session_id: "session-1".into(),
            fence: Some(WorkerFence {
                attempt_id: uuid::Uuid::nil(),
                generation: 1,
                worker_id: "worker-1".into(),
            }),
        })
        .expect("session");
        assert_eq!(encoded["sessionId"], "session-1");
        assert_eq!(encoded["generation"], 1);
        let parsed: WorkerFence = serde_json::from_value(serde_json::json!({
            "workerId": "worker-1",
            "attemptId": uuid::Uuid::nil(),
            "generation": 1,
            "workspaceRoot": "."
        }))
        .expect("legacy heartbeat extra field should be ignored");
        assert_eq!(parsed.worker_id, "worker-1");
        assert_eq!(parsed.generation, 1);
        let encoded = serde_json::to_value(WorkerPushRequest {
            force: false,
            idempotency_key: Some("worker-push-1".into()),
        })
        .expect("push");
        assert_eq!(
            encoded,
            serde_json::json!({
                "force": false,
                "idempotencyKey": "worker-push-1"
            })
        );
    }

    #[test]
    fn worker_event_request_without_cursor_remains_compatible() {
        let request: WorkerEventRequest = serde_json::from_value(serde_json::json!({
            "sourceEventId": "legacy-event",
            "eventType": "acp",
            "payload": { "ok": true }
        }))
        .expect("legacy event request should deserialize");
        assert_eq!(request.cursor, None);
        assert_eq!(request.event_type, RunEventKind::Acp);

        let encoded = serde_json::to_value(request).expect("event request should serialize");
        assert!(encoded.get("cursor").is_none());
        assert_eq!(encoded["eventType"], "acp");
    }

    #[test]
    fn worker_event_request_rejects_unknown_event_type() {
        let parsed = serde_json::from_value::<WorkerEventRequest>(serde_json::json!({
            "sourceEventId": "legacy-event",
            "eventType": "not-a-kind",
            "payload": { "ok": true }
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn worker_command_kind_round_trips_as_prompt_cancel() {
        let prompt = WorkerCommand::prompt("msg-1", "hello".into(), uuid::Uuid::nil());
        let encoded = serde_json::to_value(&prompt).expect("prompt should serialize");
        assert_eq!(encoded["kind"], "prompt");
        assert_eq!(encoded["text"], "hello");
        assert_eq!(encoded["messageId"], uuid::Uuid::nil().to_string());

        let parsed: WorkerCommand =
            serde_json::from_value(encoded).expect("prompt should deserialize");
        assert_eq!(parsed.kind, WorkerCommandKind::Prompt);
        assert_eq!(parsed.text.as_deref(), Some("hello"));

        let cancel = WorkerCommand::cancel("cancel-stopping");
        let encoded = serde_json::to_value(&cancel).expect("cancel should serialize");
        assert_eq!(encoded["kind"], "cancel");
        assert!(encoded["text"].is_null());
        assert!(encoded["messageId"].is_null());

        let parsed: WorkerCommand =
            serde_json::from_value(encoded).expect("cancel should deserialize");
        assert_eq!(parsed.kind, WorkerCommandKind::Cancel);
    }

    #[test]
    fn approval_decision_round_trips_console_strings() {
        let encoded = serde_json::to_value(ApprovalDecision::AllowOnce).expect("serialize");
        assert_eq!(encoded, serde_json::json!("allow-once"));
        let encoded = serde_json::to_value(ApprovalDecision::AllowSession).expect("serialize");
        assert_eq!(encoded, serde_json::json!("allow-always"));
        let encoded = serde_json::to_value(ApprovalDecision::Deny).expect("serialize");
        assert_eq!(encoded, serde_json::json!("reject-once"));

        let parsed: ApprovalDecision =
            serde_json::from_value(serde_json::json!("allow-once")).expect("allow-once");
        assert_eq!(parsed, ApprovalDecision::AllowOnce);
        let parsed: ApprovalDecision =
            serde_json::from_value(serde_json::json!("allow")).expect("allow alias");
        assert_eq!(parsed, ApprovalDecision::AllowSession);
        let parsed: ApprovalDecision =
            serde_json::from_value(serde_json::json!("deny")).expect("deny alias");
        assert_eq!(parsed, ApprovalDecision::Deny);
        let parsed: ApprovalDecision =
            serde_json::from_value(serde_json::json!("reject-once")).expect("reject-once");
        assert_eq!(parsed, ApprovalDecision::Deny);
    }

    #[test]
    fn create_approval_request_drops_legacy_jsonrpc_id() {
        let parsed: CreateApprovalRequest = serde_json::from_value(serde_json::json!({
            "requestKey": "permission-1",
            "jsonrpcId": "rpc-1",
            "kind": "permission",
            "payload": {
                "requestId": "permission-1",
                "rawInput": { "path": "notes.txt" }
            }
        }))
        .expect("legacy jsonrpcId should be ignored");
        let encoded = serde_json::to_value(&parsed).expect("serialize");
        assert!(encoded.get("jsonrpcId").is_none());
        assert_eq!(encoded["requestKey"], "permission-1");
        assert_eq!(encoded["kind"], "permission");
        assert_eq!(encoded["payload"]["requestId"], "permission-1");
        assert_eq!(encoded["payload"]["rawInput"]["path"], "notes.txt");
    }

    #[test]
    fn permission_mode_round_trips_snake_case() {
        for mode in [
            PermissionMode::Default,
            PermissionMode::AcceptEdits,
            PermissionMode::Yolo,
            PermissionMode::Auto,
        ] {
            let encoded = serde_json::to_value(mode).expect("serialize");
            assert_eq!(encoded, serde_json::json!(mode.as_str()));
            assert_eq!(PermissionMode::parse(mode.as_str()), Some(mode));
        }
        assert!(PermissionMode::Default.auto_resolves_approvals());
        assert!(PermissionMode::Yolo.auto_resolves_approvals());
        assert!(PermissionMode::Auto.auto_resolves_approvals());
        assert!(!PermissionMode::AcceptEdits.auto_resolves_approvals());
        assert_eq!(PermissionMode::parse("manual"), None);
    }

    #[test]
    fn message_role_round_trips_snake_case() {
        for role in [MessageRole::User, MessageRole::Assistant] {
            let encoded = serde_json::to_value(role).expect("serialize");
            assert_eq!(encoded, serde_json::json!(role.as_str()));
            assert_eq!(MessageRole::parse(role.as_str()), Some(role));
        }
        assert_eq!(MessageRole::parse("system"), None);
        let encoded = serde_json::to_value(PlatformEvent::MessageCreated {
            role: MessageRole::User,
            text: "follow-up".into(),
        })
        .expect("message");
        assert_eq!(
            encoded,
            serde_json::json!({
                "event": "message.created",
                "role": "user",
                "text": "follow-up"
            })
        );
    }

    #[test]
    fn github_mode_round_trips_snake_case() {
        for mode in [GithubMode::Mock, GithubMode::Live] {
            let encoded = serde_json::to_value(mode).expect("serialize");
            assert_eq!(encoded, serde_json::json!(mode.as_str()));
            assert_eq!(GithubMode::parse(mode.as_str()), Some(mode));
        }
        assert_eq!(GithubMode::parse("other"), None);
    }

    #[test]
    fn git_product_enums_round_trip_wire_strings() {
        for state in [
            PullRequestState::Open,
            PullRequestState::Closed,
            PullRequestState::Merged,
            PullRequestState::Draft,
        ] {
            let encoded = serde_json::to_value(state).expect("serialize");
            assert_eq!(encoded, serde_json::json!(state.as_str()));
            assert_eq!(PullRequestState::parse(state.as_str()), Some(state));
        }
        let encoded = serde_json::to_value(GithubAccountType::Organization).expect("account");
        assert_eq!(encoded, serde_json::json!("Organization"));
        assert_eq!(
            GithubAccountType::parse("User"),
            Some(GithubAccountType::User)
        );
        let encoded = serde_json::to_value(GithubInstallationStatus::Active).expect("status");
        assert_eq!(encoded, serde_json::json!("active"));
        assert_eq!(
            GithubInstallationStatus::parse("suspended"),
            Some(GithubInstallationStatus::Suspended)
        );
    }

    #[test]
    fn create_approval_request_rejects_non_product_payload() {
        let parsed = serde_json::from_value::<CreateApprovalRequest>(serde_json::json!({
            "requestKey": "permission-1",
            "kind": "permission",
            "payload": { "path": "notes.txt" }
        }));
        assert!(parsed.is_err());
    }

    #[test]
    fn approval_kind_and_risk_round_trip_wire_strings() {
        let encoded = serde_json::to_value(ApprovalKind::Permission).expect("kind");
        assert_eq!(encoded, serde_json::json!("permission"));
        let encoded = serde_json::to_value(ApprovalKind::Tool).expect("kind");
        assert_eq!(encoded, serde_json::json!("tool"));
        let encoded = serde_json::to_value(ApprovalRisk::Medium).expect("risk");
        assert_eq!(encoded, serde_json::json!("medium"));
        assert_eq!(ApprovalKind::parse("tool"), Some(ApprovalKind::Tool));
        assert_eq!(ApprovalRisk::parse("high"), Some(ApprovalRisk::High));
        assert_eq!(ApprovalKind::parse("other"), None);
        assert_eq!(ApprovalDecision::parse("unknown"), None);
    }

    #[test]
    fn cloud_event_kind_round_trips_classified_strings() {
        for kind in [
            CloudEventKind::TextDelta,
            CloudEventKind::ThoughtDelta,
            CloudEventKind::UserMessage,
            CloudEventKind::ToolCall,
            CloudEventKind::ToolResult,
            CloudEventKind::StateChanged,
            CloudEventKind::UsageUpdate,
            CloudEventKind::ProjectionReady,
            CloudEventKind::Plan,
            CloudEventKind::AvailableCommands,
            CloudEventKind::SessionStarted,
            CloudEventKind::ApprovalRequested,
            CloudEventKind::Initialized,
            CloudEventKind::UnsupportedRequest,
            CloudEventKind::TurnStarted,
            CloudEventKind::StepStarted,
            CloudEventKind::TurnEnded,
            CloudEventKind::Error,
            CloudEventKind::Acp,
        ] {
            let encoded = serde_json::to_value(kind).expect("serialize");
            assert_eq!(encoded, serde_json::json!(kind.as_event_type()));
            assert_eq!(CloudEventKind::parse(kind.as_event_type()), Some(kind));
        }
        assert_eq!(CloudEventKind::parse("platform"), None);
        assert_eq!(CloudEventKind::parse("runtime"), None);
        assert_eq!(
            RunEventKind::parse("platform"),
            Some(RunEventKind::Platform)
        );
        assert_eq!(RunEventKind::parse("runtime"), Some(RunEventKind::Runtime));
        assert_eq!(
            RunEventKind::parse("text_delta"),
            Some(RunEventKind::TextDelta)
        );
        assert_eq!(
            RunEventKind::from(CloudEventKind::ToolCall).as_event_type(),
            "tool_call"
        );
        assert_eq!(
            serde_json::to_value(RunEventKind::Platform).expect("platform"),
            serde_json::json!("platform")
        );
        assert_eq!(RunEventKind::parse("not-a-kind"), None);
    }

    #[test]
    fn classified_event_payloads_round_trip_camel_case() {
        let encoded = serde_json::to_value(TextEventPayload {
            text: "hello".into(),
        })
        .expect("text");
        assert_eq!(encoded, serde_json::json!({ "text": "hello" }));
        let encoded = serde_json::to_value(ProjectionPayload {
            source_message_count: Some(serde_json::json!(4)),
            projected_message_count: Some(serde_json::json!(3)),
            delivery: Some("strict".into()),
            context_epoch: Some(serde_json::json!(2)),
            extra: serde_json::Map::from_iter([(
                "cacheDriftDetected".into(),
                serde_json::json!(false),
            )]),
        })
        .expect("projection");
        assert_eq!(
            encoded,
            serde_json::json!({
                "sourceMessageCount": 4,
                "projectedMessageCount": 3,
                "delivery": "strict",
                "contextEpoch": 2,
                "cacheDriftDetected": false
            })
        );
        let encoded = serde_json::to_value(ToolCallPayload {
            tool_call_id: Some("call-7".into()),
            title: Some("Write".into()),
            raw_input: Some(serde_json::json!({ "path": "notes.md" })),
            ..ToolCallPayload::default()
        })
        .expect("tool");
        assert_eq!(
            encoded,
            serde_json::json!({
                "toolCallId": "call-7",
                "title": "Write",
                "rawInput": { "path": "notes.md" }
            })
        );
        assert_eq!(
            ApprovalDecision::default_allowed(),
            vec![
                ApprovalDecision::AllowOnce,
                ApprovalDecision::AllowSession,
                ApprovalDecision::Deny
            ]
        );
        let encoded = serde_json::to_value(ApprovalEventPayload {
            request_id: "call-7".into(),
            tool_call_id: Some("call-7".into()),
            title: Some("Write".into()),
            kind: Some("edit".into()),
            raw_input: Some(serde_json::json!({ "path": "notes.md" })),
            ..ApprovalEventPayload::default()
        })
        .expect("approval");
        assert_eq!(
            encoded,
            serde_json::json!({
                "requestId": "call-7",
                "toolCallId": "call-7",
                "title": "Write",
                "kind": "edit",
                "rawInput": { "path": "notes.md" }
            })
        );
        assert!(
            !ApprovalEventPayload {
                request_id: "call-7".into(),
                raw_input: Some(serde_json::json!({ "path": "notes.md" })),
                ..ApprovalEventPayload::default()
            }
            .is_ask_user()
        );
        assert!(
            ApprovalEventPayload {
                request_id: "ask-1".into(),
                raw_input: Some(serde_json::json!({
                    "askUser": true,
                    "question": "Ship this PR?"
                })),
                ..ApprovalEventPayload::default()
            }
            .is_ask_user()
        );
        let parsed: DecideApprovalRequest = serde_json::from_value(serde_json::json!({
            "decision": "allow-once",
            "optionId": "ask-0"
        }))
        .expect("ask-user decide");
        assert_eq!(parsed.decision, ApprovalDecision::AllowOnce);
        assert_eq!(parsed.option_id.as_deref(), Some("ask-0"));
        let encoded = serde_json::to_value(PlatformEvent::RunStatusChanged {
            status: RunStatus::Completed,
            head_sha: Some("abc123".into()),
        })
        .expect("status");
        assert_eq!(
            encoded,
            serde_json::json!({
                "event": "run.status",
                "status": "completed",
                "headSha": "abc123"
            })
        );
        let encoded = serde_json::to_value(PlatformEvent::RunArchived).expect("archived");
        assert_eq!(encoded, serde_json::json!({ "event": "run.archived" }));
        let encoded = serde_json::to_value(PlatformEvent::ApprovalCreated {
            approval_id: uuid::Uuid::nil(),
            status: ApprovalStatus::Pending,
            decision: None,
            kind: ApprovalKind::Tool,
        })
        .expect("approval created");
        assert_eq!(
            encoded,
            serde_json::json!({
                "event": "approval.created",
                "approvalId": uuid::Uuid::nil(),
                "status": "pending",
                "kind": "tool"
            })
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerStatusRequest {
    pub status: RunStatus,
    pub head_sha: Option<String>,
    pub failure_code: Option<String>,
    #[serde(flatten)]
    pub fence: Option<WorkerFence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerSessionRequest {
    pub session_id: String,
    #[serde(flatten)]
    pub fence: Option<WorkerFence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerTitleRequest {
    pub title: String,
    /// Optional for compatibility with legacy callers. Active worker attempts
    /// must provide the fence; the API only permits an unfenced title update
    /// when no attempt is active.
    #[serde(flatten)]
    pub fence: Option<WorkerFence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneAuthResponse {
    pub run_id: Id,
    pub repository_id: Id,
    pub clone_url: String,
    pub username: Option<String>,
    pub token: Option<String>,
    pub base_ref: String,
    pub head_branch: String,
    #[serde(default)]
    pub mock: bool,
}

/// API → worker command. Wire JSON stays `"prompt"` / `"cancel"`.
/// Approval and shutdown are not API→worker commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerCommandKind {
    Prompt,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCommand {
    pub id: String,
    pub kind: WorkerCommandKind,
    pub text: Option<String>,
    pub message_id: Option<Id>,
}

impl WorkerCommand {
    pub fn prompt(id: impl Into<String>, text: String, message_id: Id) -> Self {
        Self {
            id: id.into(),
            kind: WorkerCommandKind::Prompt,
            text: Some(text),
            message_id: Some(message_id),
        }
    }

    pub fn cancel(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            kind: WorkerCommandKind::Cancel,
            text: None,
            message_id: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCommandsResponse {
    pub commands: Vec<WorkerCommand>,
    /// Pending ACP session mode, if any. Worker applies via
    /// `RuntimeCommand::SetMode` while idle. Not a [`WorkerCommandKind`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode_id: Option<String>,
    /// Current run title so the worker can skip auto-refresh after a user rename.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCommandAckRequest {
    pub message_id: Id,
    #[serde(flatten)]
    pub fence: WorkerFence,
}

/// Product approval outcome stored by Cloud and sent by Console.
/// Wire JSON stays `allow-once` / `allow-always` / `reject-once`.
/// ACP `optionId` mapping stays in the runtime-client adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalDecision {
    #[serde(rename = "allow-once")]
    AllowOnce,
    #[serde(rename = "allow-always", alias = "allow")]
    AllowSession,
    #[serde(rename = "reject-once", alias = "deny")]
    Deny,
}

impl ApprovalDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllowOnce => "allow-once",
            Self::AllowSession => "allow-always",
            Self::Deny => "reject-once",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "allow-once" => Self::AllowOnce,
            "allow-always" | "allow" => Self::AllowSession,
            "reject-once" | "deny" => Self::Deny,
            _ => return None,
        })
    }

    pub fn default_allowed() -> Vec<Self> {
        vec![Self::AllowOnce, Self::AllowSession, Self::Deny]
    }

    pub fn status(self) -> ApprovalStatus {
        match self {
            Self::Deny => ApprovalStatus::Denied,
            Self::AllowOnce | Self::AllowSession => ApprovalStatus::Approved,
        }
    }
}

/// Product approval kind stored by Cloud. Wire JSON stays `permission` / `tool`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalKind {
    Permission,
    Tool,
}

impl ApprovalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Permission => "permission",
            Self::Tool => "tool",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "permission" => Some(Self::Permission),
            "tool" => Some(Self::Tool),
            _ => None,
        }
    }
}

/// Product approval risk stored by Cloud. Wire JSON stays `low` / `medium` / `high`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRisk {
    Low,
    Medium,
    High,
}

impl ApprovalRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApprovalRequest {
    pub request_key: String,
    pub kind: ApprovalKind,
    #[serde(default = "default_risk")]
    pub risk: ApprovalRisk,
    /// Product payload written by the worker. Wire JSON stays camelCase.
    pub payload: ApprovalEventPayload,
    #[serde(default = "default_allowed_decisions")]
    pub allowed_decisions: Vec<ApprovalDecision>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

fn default_risk() -> ApprovalRisk {
    ApprovalRisk::Medium
}

fn default_allowed_decisions() -> Vec<ApprovalDecision> {
    ApprovalDecision::default_allowed()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    Resolved,
    Expired,
    Cancelled,
}

impl ApprovalStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Denied => "denied",
            Self::Resolved => "resolved",
            Self::Expired => "expired",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "approved" => Self::Approved,
            "denied" => Self::Denied,
            "resolved" => Self::Resolved,
            "expired" => Self::Expired,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: Id,
    pub run_id: Id,
    pub request_key: String,
    pub kind: ApprovalKind,
    pub risk: ApprovalRisk,
    /// Stored JSON. New rows are `ApprovalEventPayload`; historical ACP
    /// envelopes still load as raw values.
    pub payload: serde_json::Value,
    pub status: ApprovalStatus,
    pub allowed_decisions: Vec<ApprovalDecision>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub decision: Option<ApprovalDecision>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideApprovalRequest {
    pub decision: ApprovalDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveApprovalRequest {
    pub decision: ApprovalDecision,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerPushRequest {
    #[serde(default)]
    pub force: bool,
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerPullRequestRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    #[serde(default = "default_draft")]
    pub draft: bool,
    pub idempotency_key: Option<String>,
}

fn default_draft() -> bool {
    true
}

// --- GitHub / Git Broker ---

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GithubMode {
    Mock,
    Live,
}

impl GithubMode {
    pub fn from_env() -> Self {
        match std::env::var("ZENE_CLOUD_GITHUB_MODE")
            .unwrap_or_else(|_| "live".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "mock" => Self::Mock,
            _ => Self::Live,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Mock => "mock",
            Self::Live => "live",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "mock" => Self::Mock,
            "live" => Self::Live,
            _ => return None,
        })
    }
}

/// GitHub account kind stored on installations. Wire JSON stays `User` / `Organization`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum GithubAccountType {
    User,
    #[default]
    Organization,
}

impl GithubAccountType {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "User",
            Self::Organization => "Organization",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "User" | "user" => Self::User,
            "Organization" | "organization" => Self::Organization,
            _ => return None,
        })
    }
}

/// GitHub App installation status. Wire JSON stays snake_case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GithubInstallationStatus {
    #[default]
    Active,
    Suspended,
}

impl GithubInstallationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Suspended => "suspended",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "active" => Self::Active,
            "suspended" => Self::Suspended,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubAccount {
    pub id: Id,
    pub user_id: Id,
    pub github_user_id: String,
    pub login: String,
    pub access_token_enc: String,
    pub token_type: String,
    pub scope: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Public GitHub account. Never includes `access_token_enc`.
pub fn github_account_view(account: &GithubAccount) -> serde_json::Value {
    serde_json::json!({
        "id": account.id,
        "userId": account.user_id,
        "githubUserId": account.github_user_id,
        "login": account.login,
        "tokenType": account.token_type,
        "scope": account.scope,
        "createdAt": account.created_at,
        "updatedAt": account.updated_at,
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubInstallation {
    pub id: Id,
    pub organization_id: Id,
    pub installation_id: String,
    pub account_login: String,
    pub account_type: GithubAccountType,
    pub status: GithubInstallationStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OauthState {
    pub state: String,
    pub user_id: Option<Id>,
    pub redirect_to: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperationKind {
    Clone,
    PushBundle,
    CreatePr,
    SyncRepos,
}

impl GitOperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clone => "clone",
            Self::PushBundle => "push_bundle",
            Self::CreatePr => "create_pr",
            Self::SyncRepos => "sync_repos",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "clone" => Self::Clone,
            "push_bundle" => Self::PushBundle,
            "create_pr" => Self::CreatePr,
            "sync_repos" => Self::SyncRepos,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GitOperationStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

impl GitOperationStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "pending" => Self::Pending,
            "running" => Self::Running,
            "succeeded" => Self::Succeeded,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitOperation {
    pub id: Id,
    pub organization_id: Id,
    pub repository_id: Id,
    pub run_id: Id,
    pub operation: GitOperationKind,
    pub expected_head_sha: Option<String>,
    pub result_head_sha: Option<String>,
    pub approval_id: Option<Id>,
    pub status: GitOperationStatus,
    pub idempotency_key: String,
    pub provider_request_id: Option<String>,
    pub result: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
}

/// Stored pull-request lifecycle. Mock rows may use `draft`; GitHub live rows use `open` / `closed`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PullRequestState {
    Open,
    Closed,
    Merged,
    Draft,
}

impl PullRequestState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Merged => "merged",
            Self::Draft => "draft",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "open" => Self::Open,
            "closed" => Self::Closed,
            "merged" => Self::Merged,
            "draft" => Self::Draft,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PullRequest {
    pub id: Id,
    pub repository_id: Id,
    pub run_id: Id,
    pub provider_number: Option<i64>,
    pub url: Option<String>,
    pub title: String,
    pub body: Option<String>,
    pub base_sha: Option<String>,
    pub head_sha: Option<String>,
    pub state: PullRequestState,
    pub draft: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditLog {
    pub id: Id,
    pub organization_id: Option<Id>,
    pub actor_type: String,
    pub actor_id: Option<String>,
    pub action: String,
    pub resource_type: Option<String>,
    pub resource_id: Option<String>,
    pub metadata: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRepoSummary {
    pub provider_repo_id: String,
    pub owner: String,
    pub name: String,
    pub default_branch: String,
    pub clone_url: String,
    pub private: bool,
    pub installation_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubBranchSummary {
    pub name: String,
    #[serde(default)]
    pub default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubUser {
    pub id: String,
    pub login: String,
    pub name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartGithubOauthRequest {
    pub redirect_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartGithubOauthResponse {
    pub authorize_url: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubOauthCallbackRequest {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpsertInstallationRequest {
    pub installation_id: String,
    pub account_login: String,
    #[serde(default)]
    pub account_type: GithubAccountType,
    #[serde(default)]
    pub status: GithubInstallationStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubProviderConfig {
    pub organization_id: Id,
    #[serde(default = "default_github_mode")]
    pub mode: GithubMode,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub app_id: Option<String>,
    pub app_private_key: Option<String>,
    pub app_slug: Option<String>,
    pub updated_at: DateTime<Utc>,
}

fn default_github_mode() -> GithubMode {
    GithubMode::Live
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubProviderConfigView {
    pub mode: GithubMode,
    pub configured: bool,
    pub client_id: Option<String>,
    pub has_client_secret: bool,
    pub app_id: Option<String>,
    pub has_app_private_key: bool,
    pub app_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGithubProviderConfigRequest {
    #[serde(default)]
    pub mode: Option<GithubMode>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub app_id: Option<String>,
    pub app_private_key: Option<String>,
    pub app_slug: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatePullRequestBody {
    pub title: String,
    pub body: Option<String>,
    #[serde(default = "default_draft")]
    pub draft: bool,
    pub base_ref: Option<String>,
    pub head_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptBundleRequest {
    pub expected_head_sha: Option<String>,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptBundleResult {
    pub head_sha: String,
    pub push_url: String,
    pub operation_id: Id,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloneTokenResponse {
    pub token: String,
    pub clone_url: String,
    pub expires_at: DateTime<Utc>,
    pub mode: GithubMode,
}

/// User-facing BYOK settings (never includes full api_key).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmSettingsView {
    pub provider_id: String,
    pub base_url: String,
    pub default_model: String,
    pub models: Vec<String>,
    pub has_api_key: bool,
    pub api_key_hint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLlmSettingsRequest {
    pub provider_id: String,
    pub base_url: String,
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<String>,
    /// Omit or empty to keep the existing key.
    pub api_key: Option<String>,
}

/// Stored user LLM provider endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserLlmProvider {
    pub id: Id,
    pub user_id: Id,
    pub provider_id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub default_model: String,
    pub models: Vec<String>,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmProviderView {
    pub id: Id,
    pub provider_id: String,
    pub name: String,
    pub base_url: String,
    pub default_model: String,
    pub models: Vec<String>,
    pub has_api_key: bool,
    pub api_key_hint: Option<String>,
    pub is_default: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateLlmProviderRequest {
    pub provider_id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub models: Vec<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLlmProviderRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub default_model: Option<String>,
    #[serde(default)]
    pub models: Option<Vec<String>>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub is_default: Option<bool>,
}

impl UserLlmProvider {
    pub fn to_view(&self) -> LlmProviderView {
        let trimmed = self.api_key.trim();
        let has_api_key = !trimmed.is_empty();
        let api_key_hint = if has_api_key {
            let hint: String = trimmed
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            Some(format!("••••{hint}"))
        } else {
            None
        };
        LlmProviderView {
            id: self.id,
            provider_id: self.provider_id.clone(),
            name: self.name.clone(),
            base_url: self.base_url.clone(),
            default_model: self.default_model.clone(),
            models: self.models.clone(),
            has_api_key,
            api_key_hint,
            is_default: self.is_default,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// Stored row (includes api_key for worker injection).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserLlmSettings {
    pub user_id: Id,
    pub provider_id: String,
    pub base_url: String,
    pub api_key: String,
    pub default_model: String,
    pub models: Vec<String>,
    pub updated_at: DateTime<Utc>,
}

/// Internal worker credential payload for spawning `zene acp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LlmAuthResponse {
    pub api_key: String,
    pub base_url: String,
    pub model: String,
    /// Wire protocol for Zene (`openai` / openai-compatible).
    pub provider: String,
}

impl UserLlmSettings {
    pub fn to_view(&self) -> LlmSettingsView {
        let trimmed = self.api_key.trim();
        let has_api_key = !trimmed.is_empty();
        let api_key_hint = if has_api_key {
            let hint: String = trimmed
                .chars()
                .rev()
                .take(4)
                .collect::<String>()
                .chars()
                .rev()
                .collect();
            Some(format!("••••{hint}"))
        } else {
            None
        };
        LlmSettingsView {
            provider_id: self.provider_id.clone(),
            base_url: self.base_url.clone(),
            default_model: self.default_model.clone(),
            models: self.models.clone(),
            has_api_key,
            api_key_hint,
        }
    }
}

/// Immediate session title from a user prompt: a short topic, not the request dump.
pub fn summarize_prompt_title(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .trim_start_matches(['#', '-', '*', ' '])
        .trim();
    if line.is_empty() {
        return "Untitled agent".into();
    }

    let mut s = line.to_string();
    for prefix in [
        "请你帮我",
        "请帮我",
        "请你",
        "帮我",
        "麻烦",
        "请深入分析",
        "请分析一下",
        "请分析",
        "深入分析",
        "请用 Rust 逐步",
        "请用Rust逐步",
        "请用 Rust ",
        "请用Rust",
        "请用 ",
        "请",
        "Please use Rust to step-by-step ",
        "Please use Rust to ",
        "Please ",
        "please ",
    ] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start().to_string();
            break;
        }
    }
    for prefix in ["逐步", "分析"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start().to_string();
        }
    }
    for sep in [
        "时，为什么",
        "时为什么",
        "？请给出",
        "？请",
        "为什么",
        "，并给出",
        "，并提供",
        "，并设计",
        "，并",
        "，以及",
        ", and provide",
        ", and give",
        ", and ",
    ] {
        if let Some((head, _)) = s.split_once(sep) {
            s = head.trim().to_string();
            break;
        }
    }
    for sep in ["在面对", "在遭遇"] {
        if let Some((head, _)) = s.split_once(sep) {
            let head = head.trim();
            if !head.is_empty() {
                s = head.to_string();
            }
            break;
        }
    }
    for suffix in ["怎么样", "如何", "吗", "呢"] {
        if let Some(rest) = s.strip_suffix(suffix) {
            s = rest.trim_end().to_string();
        }
    }
    s = s
        .trim()
        .trim_end_matches(['。', '.', '，', ',', '？', '?', '！', '!'])
        .trim()
        .to_string();
    if s.is_empty() {
        s = line.to_string();
    }
    let mut title: String = s.chars().take(28).collect();
    if s.chars().count() > 28 {
        title.push('…');
    }
    if title.is_empty() {
        "Untitled agent".into()
    } else {
        title
    }
}

/// True when `title` is just the prompt (or its truncated prefix), not a summary.
pub fn title_is_prompt_echo(title: &str, prompt: &str) -> bool {
    let title_n: String = title
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    let prompt_n: String = prompt
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect();
    if title_n.is_empty() || prompt_n.is_empty() {
        return title_n.is_empty();
    }
    if title_n == prompt_n {
        return true;
    }
    let n = title_n.chars().count();
    n >= 8 && prompt_n.starts_with(&title_n)
}

#[cfg(test)]
mod title_tests {
    use super::{summarize_prompt_title, title_is_prompt_echo};

    #[test]
    fn summarizes_long_chinese_request() {
        let prompt = "请用 Rust 逐步分析多线程 Tokio 异步队列可能出现的死锁根因，并给出推导证明与完整的重构代码";
        let title = summarize_prompt_title(prompt);
        assert_ne!(title, prompt);
        assert!(!title.starts_with("请用"));
        assert!(title.contains("Tokio") || title.contains("死锁") || title.contains("异步队列"));
        assert!(title.chars().count() <= 29);
        assert!(title_is_prompt_echo(
            "请用 Rust 逐步分析多线程 Tokio 异步队列可…",
            prompt
        ));
        assert!(!title_is_prompt_echo("Tokio异步队列死锁根因分析", prompt));
    }

    #[test]
    fn strips_question_tail() {
        assert_eq!(summarize_prompt_title("sglang 目前性能怎么样"), "sglang 目前性能");
    }

    #[test]
    fn summarizes_deep_analysis_request() {
        let prompt = "请深入分析 Raft 共识算法在面对不对称网络分区 (Asymmetric Network Partition) 和脑裂边缘场景时，为什么单纯依赖 Term 递增可能引发幽灵日志写入 (Phantom Log Writes) 或提交回滚？请给出形式化状态机推导证明，并设计一个防范该边缘 Bug 的 Pre-Vote 状态迁移算法与 Rust 实现。";
        let title = summarize_prompt_title(prompt);
        assert!(!title.starts_with("请"));
        assert!(title.contains("Raft"));
        assert!(!title.contains("请给出"));
        assert!(title.chars().count() <= 29);
        assert_ne!(title, prompt.chars().take(56).collect::<String>());
    }
}
