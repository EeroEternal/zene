export interface User {
  id?: string;
  email?: string;
  displayName?: string;
}

export interface Organization {
  id?: string;
  name?: string;
}

export interface GithubAccount {
  login?: string;
}

export interface GithubInstallation {
  accountLogin?: string;
}

export interface GithubStatus {
  mode?: string;
  configured?: boolean;
  connected?: boolean;
  account?: GithubAccount | null;
  installations?: GithubInstallation[];
  installUrl?: string | null;
  displayLogin?: string | null;
  hint?: string;
}

export interface Repo {
  id: string;
  owner: string;
  name: string;
  defaultBranch?: string;
}

export interface Branch {
  name: string;
  default?: boolean;
}

export interface Run {
  id: string;
  title: string;
  status: string;
  repositoryId: string;
  headBranch?: string;
  baseRef?: string;
  model?: string;
  permissionMode?: string;
  /** Agent step budget; `0` = unlimited. */
  maxTurns?: number;
  headSha?: string;
  createdAt?: string;
  updatedAt?: string;
  startedAt?: string;
  archivedAt?: string;
}

export interface RunMessage {
  role: string;
  content: string;
  createdAt: string;
}

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

export interface RunEvent {
  seq: number;
  /** Optional provider/runtime cursor; `seq` remains the server ordering. */
  cursor?: number;
  createdAt: string;
  eventType?: string;
  event_type?: string;
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
    params?: {
      update?: AcpSessionUpdate;
    };
  };
}

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
  payload?: unknown;
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

export interface PullRequest {
  title: string;
  url?: string;
  providerNumber?: number;
  state: string;
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
export type PermissionMode = "default" | "accept_edits" | "yolo";
export type View = "new" | "settings" | "run";

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
