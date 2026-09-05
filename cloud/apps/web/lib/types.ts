export interface User {
  id?: string;
  email?: string;
  displayName?: string;
}

export interface Organization {
  id?: string;
  name?: string;
}

export interface AuthResponse {
  token: string;
  user: User;
  organization: Organization;
}

export interface LoginRequest {
  email: string;
  password: string;
}

export interface RegisterRequest {
  email: string;
  password: string;
  displayName?: string;
  code?: string;
}

export interface SendVerificationCodeRequest {
  email: string;
  purpose?: "register" | "reset_password";
}

export interface SendVerificationCodeResponse {
  ok: boolean;
  code?: string;
}

export interface ResetPasswordRequest {
  email: string;
  code: string;
  newPassword: string;
}

export interface GithubAccount {
  login?: string;
}

export interface GithubInstallation {
  accountLogin?: string;
  accountType?: GithubAccountType;
  status?: GithubInstallationStatus;
}

/** GitHub integration mode. Matches domain `GithubMode`. */
export type GithubMode = "mock" | "live";

/** GitHub account kind stored on installations. Matches domain `GithubAccountType`. */
export type GithubAccountType = "User" | "Organization";

/** GitHub App installation status. Matches domain `GithubInstallationStatus`. */
export type GithubInstallationStatus = "active" | "suspended";

export interface GithubStatus {
  mode?: GithubMode;
  configured?: boolean;
  connected?: boolean;
  account?: GithubAccount | null;
  installations?: GithubInstallation[];
  installUrl?: string | null;
  displayLogin?: string | null;
  hint?: string;
}

export interface GithubProviderConfigView {
  mode?: GithubMode;
  configured?: boolean;
  appId?: string | null;
  hasAppPrivateKey?: boolean;
  appSlug?: string | null;
}

export interface GithubSettingsView {
  provider?: GithubProviderConfigView;
  connected?: boolean;
  installUrl?: string | null;
  redirectUri?: string | null;
}

export interface Repo {
  id: string;
  owner: string;
  name: string;
  defaultBranch?: string;
}

/** Matches domain `CreateRepositoryRequest`. */
export interface CreateRepositoryRequest {
  owner: string;
  name: string;
  defaultBranch?: string;
  cloneUrl?: string;
}

export interface Branch {
  name: string;
  default?: boolean;
}

/** Stored run lifecycle. Matches domain `RunStatus`. */
export type RunStatus =
  | "created"
  | "queued"
  | "provisioning"
  | "cloning"
  | "starting"
  | "running"
  | "waiting_for_approval"
  | "waiting_for_user"
  | "stopping"
  | "completed"
  | "failed"
  | "timed_out"
  | "cancelled";

/** Stored run permission mode. `auto` is historical; the composer picker does not offer it. */
export type PermissionMode = "default" | "accept_edits" | "yolo" | "auto";

/** Stored chat turn author. Matches domain `MessageRole`. */
export type MessageRole = "user" | "assistant";

export interface Run {
  id: string;
  title: string;
  status: RunStatus;
  repositoryId: string;
  workspaceId?: string;
  headBranch?: string;
  baseRef?: string;
  model?: string;
  permissionMode?: PermissionMode;
  /** Agent step budget; `0` = unlimited. */
  maxTurns?: number;
  headSha?: string;
  createdAt?: string;
  updatedAt?: string;
  startedAt?: string;
  archivedAt?: string;
}

/** Matches domain `CreateRunRequest`. */
export interface CreateRunRequest {
  repositoryId: string;
  prompt: string;
  baseRef?: string;
  model?: string;
  permissionMode?: PermissionMode;
  maxTurns?: number;
  modeId?: string;
}

export interface RunMessage {
  role: MessageRole;
  content: string;
  createdAt: string;
}

/** Classified product kinds written by RuntimeClient. */
export type CloudEventKind =
  | "text_delta"
  | "thought_delta"
  | "user_message"
  | "tool_call"
  | "tool_result"
  | "state_changed"
  | "usage_update"
  | "projection_ready"
  | "plan"
  | "available_commands"
  | "session_started"
  | "approval_requested"
  | "initialized"
  | "unsupported_request"
  | "turn_started"
  | "step_started"
  | "turn_ended"
  | "error"
  | "acp";

/** Stored `event_type` written by Cloud. Matches domain `RunEventKind`. */
export type RunEventType = CloudEventKind | "platform" | "runtime";

export interface AcpSessionUpdate {
  sessionUpdate?: string;
  content?: { text?: string } | Array<{ type?: string; content?: { text?: string }; text?: string }>;
  title?: string;
  toolName?: string;
  toolCallId?: string;
  kind?: string;
  status?: string;
  rawInput?: unknown;
  rawOutput?: { text?: string; isError?: boolean };
}

export interface TextEventPayload {
  text?: string;
}

export interface ToolCallPayload {
  toolCallId?: string;
  title?: string;
  toolName?: string;
  kind?: string;
  status?: string;
  rawInput?: unknown;
}

export interface ToolResultPayload {
  toolCallId?: string;
  title?: string;
  toolName?: string;
  kind?: string;
  status?: string;
  rawOutput?: { text?: string; isError?: boolean };
  text?: string;
  isError?: boolean;
}

export interface SessionStartedPayload {
  sessionId?: string;
  resumed?: boolean;
  currentModeId?: string;
  availableModes?: unknown;
  /** Inspect-only; never used to auto-resume or replay pending tools. */
  recovery?: SessionRecoveryPayload;
}

export interface SessionRecoveryPayload {
  disposition?: string;
  hasIncompleteExecution?: boolean;
  activeTurnCount?: number;
  activeToolCount?: number;
  safeResumeAllowed?: boolean;
  automaticResume?: boolean;
  reason?: string;
}

export interface InitializedPayload {
  protocolVersion?: unknown;
  agentCapabilities?: unknown;
  agentInfo?: unknown;
  authMethods?: unknown;
  [key: string]: unknown;
}

export interface UnsupportedRequestPayload {
  method: string;
  params?: unknown;
}

export interface AcpResidualPayload {
  method?: string;
  sessionUpdate?: string;
  update?: unknown;
}

export interface TurnStartedPayload {
  turnId?: string;
}

export interface StepStartedPayload {
  step?: number;
  turnId?: string;
}

export interface TurnEndedPayload {
  steps?: number;
  turnId?: string;
}

export interface ErrorPayload {
  message: string;
}

export interface StateChangedPayload {
  state?: unknown;
}

export interface UsagePayload {
  used?: unknown;
  size?: unknown;
  promptTokens?: unknown;
  completionTokens?: unknown;
  contextPercent?: unknown;
  contextEpoch?: unknown;
  cachedTokens?: unknown;
}

export interface PlanPayload {
  entries?: unknown;
}

export interface ProjectionPayload {
  sourceMessageCount?: unknown;
  projectedMessageCount?: unknown;
  delivery?: string;
  contextEpoch?: unknown;
}

export interface AvailableCommandsPayload {
  availableCommands?: unknown;
}

export interface ApprovalEventPayload {
  requestId: string;
  toolCallId?: string;
  title?: string;
  toolName?: string;
  kind?: string;
  status?: string;
  rawInput?: unknown;
}

export interface RunEvent {
  seq: number;
  /** Optional provider/runtime cursor; `seq` remains the server ordering. */
  cursor?: number;
  createdAt: string;
  eventType?: RunEventType;
  event_type?: RunEventType;
  payload?: {
    event?: string;
    status?: string;
    headSha?: string;
    prompt?: string;
    role?: string;
    text?: string;
    title?: string;
    toolCallId?: string;
    toolName?: string;
    kind?: string;
    rawInput?: unknown;
    rawOutput?: { text?: string; isError?: boolean };
    isError?: boolean;
    requestId?: string;
    sessionId?: string;
    resumed?: boolean;
    params?: {
      update?: AcpSessionUpdate;
    };
  };
}

export type PlatformEvent =
  | { event: "run.created"; title?: string; prompt?: string }
  | { event: "run.title"; title?: string }
  | { event: "run.archived" }
  | { event: "run.status"; status?: RunStatus; headSha?: string }
  | { event: "message.created"; role?: MessageRole; text?: string }
  | {
      event: "approval.created";
      approvalId?: string;
      status?: ApprovalStatus;
      decision?: ApprovalDecision | null;
      kind?: ApprovalKind;
    }
  | { event: "approval.decided"; approvalId?: string; decision?: ApprovalDecision };

export type ApprovalDecision =
  | "allow-once"
  | "allow-always"
  | "reject-once"
  | "allow"
  | "deny";
export type ApprovalKind = "permission" | "tool";
export type ApprovalRisk = "low" | "medium" | "high";
export type ApprovalStatus =
  | "pending"
  | "approved"
  | "denied"
  | "resolved"
  | "expired"
  | "cancelled";

export interface Approval {
  id: string;
  kind?: ApprovalKind;
  risk?: ApprovalRisk;
  status?: ApprovalStatus;
  /** Product fields on new rows; legacy ACP envelopes may still include `params` / `method`. */
  payload?: ApprovalEventPayload & { params?: unknown; method?: unknown };
  allowedDecisions?: ApprovalDecision[];
}

export interface WorkspaceFile {
  path: string;
  kind: "file" | "dir";
  size?: number;
}

export interface GitStatusFile {
  path: string;
  /** M / A / D / R / C / U / ? */
  status: string;
  additions: number;
  deletions: number;
}

export interface GitStatus {
  files: GitStatusFile[];
  totalAdditions: number;
  totalDeletions: number;
}

export interface GitCompare {
  base: string;
  head: string;
  files: GitStatusFile[];
  totalAdditions: number;
  totalDeletions: number;
}

export interface GitCommit {
  sha: string;
  shortSha: string;
  subject: string;
  author: string;
  authoredAt: string;
}

/** Stored pull-request lifecycle. Matches domain `PullRequestState`. */
export type PullRequestState = "open" | "closed" | "merged" | "draft";

export interface PullRequest {
  id: string;
  title: string;
  url?: string;
  providerNumber?: number;
  state: PullRequestState;
  draft?: boolean;
}

export interface McpServer {
  id: string;
  name: string;
  enabled: boolean;
  needsLogin: boolean;
}

export interface Skill {
  id: string;
  label: string;
  insert: string;
}

export type ListGroup = "project" | "date" | "status" | "none";
export type ListFilter = "none" | "running" | "completed" | "failed" | "project";
export type View = "new" | "settings" | "run";

export interface LlmProviderView {
  id: string;
  providerId: string;
  name: string;
  baseUrl: string;
  defaultModel: string;
  models: string[];
  hasApiKey: boolean;
  apiKeyHint?: string | null;
  isDefault: boolean;
  createdAt: string;
  updatedAt: string;
}

export interface CreateLlmProviderRequest {
  providerId: string;
  name?: string;
  baseUrl: string;
  defaultModel?: string;
  models?: string[];
  apiKey?: string;
  isDefault?: boolean;
}

export interface UpdateLlmProviderRequest {
  providerId?: string;
  name?: string;
  baseUrl?: string;
  defaultModel?: string;
  models?: string[];
  apiKey?: string;
  isDefault?: boolean;
}

export interface LlmSettingsView {
  providerId: string;
  baseUrl: string;
  defaultModel: string;
  models: string[];
  hasApiKey: boolean;
  apiKeyHint?: string | null;
}

export interface UpdateLlmSettingsRequest {
  providerId: string;
  baseUrl: string;
  defaultModel: string;
  models: string[];
  apiKey?: string;
}
