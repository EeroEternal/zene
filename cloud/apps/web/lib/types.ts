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
  headSha?: string;
  createdAt?: string;
  updatedAt?: string;
  startedAt?: string;
}

export interface RunMessage {
  role: string;
  content: string;
}

export interface RunEvent {
  seq: number;
  eventType?: string;
  event_type?: string;
  payload?: {
    event?: string;
    status?: string;
    headSha?: string;
    params?: {
      update?: {
        sessionUpdate?: string;
        content?: { text?: string };
        title?: string;
        toolName?: string;
        status?: string;
      };
    };
  };
}

export interface Approval {
  id: string;
  kind?: string;
  risk?: string;
  status?: string;
  payload?: unknown;
  allowedDecisions?: string[];
}

export interface WorkspaceFile {
  path: string;
  kind: "file" | "dir";
  size?: number;
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
