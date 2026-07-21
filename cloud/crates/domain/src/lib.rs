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
        matches!(
            self,
            Self::Failed | Self::TimedOut | Self::Cancelled
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Run {
    pub id: Id,
    pub organization_id: Id,
    pub repository_id: Id,
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
    pub permission_mode: String,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub run_id: Id,
    pub seq: i64,
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
    pub role: String,
    pub content: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRunRequest {
    pub repository_id: Id,
    pub prompt: String,
    #[serde(default = "default_base_ref")]
    pub base_ref: String,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
}

fn default_base_ref() -> String {
    "main".into()
}

fn default_model() -> String {
    "default".into()
}

fn default_permission_mode() -> String {
    "default".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostMessageRequest {
    pub text: String,
    pub client_message_id: Option<String>,
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
    #[serde(default = "default_base_ref")]
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
    pub workspace_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerEventRequest {
    pub source_event_id: String,
    pub event_type: String,
    pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerStatusRequest {
    pub status: RunStatus,
    pub head_sha: Option<String>,
    pub failure_code: Option<String>,
}
