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
    #[serde(default)]
    pub base_ref: Option<String>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_permission_mode")]
    pub permission_mode: String,
    /// Agent step budget; `0` = unlimited. Defaults to 50.
    #[serde(default = "default_max_turns")]
    pub max_turns: u32,
}

fn default_branch_name() -> String {
    "main".into()
}

fn default_model() -> String {
    "default".into()
}

fn default_permission_mode() -> String {
    "default".into()
}

fn default_max_turns() -> u32 {
    50
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
    /// Existing ACP session to resume after a worker replacement, if available.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_session_id: Option<String>,
    pub workspace_dir: String,
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
    pub status: String,
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
    pub event_type: String,
    pub payload: serde_json::Value,
    #[serde(flatten)]
    pub fence: Option<WorkerFence>,
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
pub struct WorkerFenceRequest {
    pub attempt_id: Id,
    pub generation: i64,
    pub worker_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerAcpSessionRequest {
    pub session_id: String,
    #[serde(flatten)]
    pub fence: Option<WorkerFence>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerTitleRequest {
    pub title: String,
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
    /// When true the worker should `git init` a local sample workspace instead of cloning.
    pub mock: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCommand {
    pub id: String,
    /// `prompt` | `cancel`
    pub kind: String,
    pub text: Option<String>,
    pub message_id: Option<Id>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerCommandsResponse {
    pub commands: Vec<WorkerCommand>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateApprovalRequest {
    pub request_key: String,
    pub jsonrpc_id: Option<String>,
    pub kind: String,
    #[serde(default = "default_risk")]
    pub risk: String,
    pub payload: serde_json::Value,
    #[serde(default = "default_allowed_decisions")]
    pub allowed_decisions: Vec<String>,
    #[serde(default)]
    pub expires_at: Option<DateTime<Utc>>,
}

fn default_risk() -> String {
    "medium".into()
}

fn default_allowed_decisions() -> Vec<String> {
    vec![
        "allow-once".into(),
        "allow-always".into(),
        "reject-once".into(),
        "allow".into(),
        "deny".into(),
    ]
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
    pub jsonrpc_id: Option<String>,
    pub kind: String,
    pub risk: String,
    pub payload: serde_json::Value,
    pub status: ApprovalStatus,
    pub allowed_decisions: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<String>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub decision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecideApprovalRequest {
    pub decision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolveApprovalRequest {
    pub decision: String,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubInstallation {
    pub id: Id,
    pub organization_id: Id,
    pub installation_id: String,
    pub account_login: String,
    pub account_type: String,
    pub status: String,
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
    pub state: String,
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
    #[serde(default = "default_account_type")]
    pub account_type: String,
    #[serde(default = "default_installation_status")]
    pub status: String,
}

fn default_account_type() -> String {
    "Organization".into()
}

fn default_installation_status() -> String {
    "active".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubProviderConfig {
    pub organization_id: Id,
    pub mode: GithubMode,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub app_id: Option<String>,
    pub app_private_key: Option<String>,
    pub app_slug: Option<String>,
    pub updated_at: DateTime<Utc>,
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
    pub mode: String,
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
