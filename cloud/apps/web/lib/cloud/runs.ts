import type {
  Approval,
  ApprovalDecision,
  CreateRunRequest,
  GitCommit,
  GitCompare,
  PullRequest,
  Run,
  RunEvent,
  RunMessage,
  WorkspaceFile,
} from "@/lib/types";
import { deleteJson, getJson, patchJson, postJson } from "./http";

export const runsApi = {
  list: () => getJson<Run[]>("/api/v1/runs"),
  create: (body: CreateRunRequest) => postJson<Run>("/api/v1/runs", body),
  get: (runId: string) => getJson<Run>(`/api/v1/runs/${runId}`),
  update: (runId: string, body: { title?: string; archived?: boolean }) =>
    patchJson<Run>(`/api/v1/runs/${runId}`, body),
  remove: (runId: string) => deleteJson(`/api/v1/runs/${runId}`),
  messages: (runId: string) => getJson<RunMessage[]>(`/api/v1/runs/${runId}/messages`),
  postMessage: (runId: string, text: string, clientMessageId = crypto.randomUUID()) =>
    postJson(`/api/v1/runs/${runId}/messages`, { text, clientMessageId }),
  events: (runId: string, afterSeq: number) =>
    getJson<{ events?: RunEvent[]; nextSeq?: number }>(
      `/api/v1/runs/${runId}/events?afterSeq=${afterSeq}`,
    ),
  cancel: (runId: string) => postJson<Run>(`/api/v1/runs/${runId}/cancel`),
  retry: (runId: string, text?: string) =>
    postJson<Run>(`/api/v1/runs/${runId}/retry`, text ? { text } : {}),
  approvals: (runId: string) => getJson<Approval[]>(`/api/v1/runs/${runId}/approvals`),
  decideApproval: (
    runId: string,
    approvalId: string,
    decision: ApprovalDecision,
    extra?: { optionId?: string; answer?: string },
  ) =>
    postJson(`/api/v1/runs/${runId}/approvals/${approvalId}/decide`, {
      decision,
      optionId: extra?.optionId,
      answer: extra?.answer,
    }),
  files: (runId: string) => getJson<WorkspaceFile[]>(`/api/v1/runs/${runId}/files`),
  file: (runId: string, path: string) =>
    getJson<{ path: string; content?: string; truncated?: boolean }>(
      `/api/v1/runs/${runId}/file?path=${encodeURIComponent(path)}`,
    ),
  gitCompare: (runId: string) => getJson<GitCompare>(`/api/v1/runs/${runId}/git/compare`),
  gitCompareDiff: (runId: string, path: string) =>
    getJson<{ diff?: string }>(
      `/api/v1/runs/${runId}/git/compare/diff?path=${encodeURIComponent(path)}`,
    ),
  gitCommits: (runId: string) => getJson<GitCommit[]>(`/api/v1/runs/${runId}/git/commits`),
  pullRequests: (runId: string) => getJson<PullRequest[]>(`/api/v1/runs/${runId}/pull-requests`),
  createPullRequest: (
    runId: string,
    body: { title: string; body?: string; draft?: boolean; baseRef?: string; headRef?: string },
  ) => postJson<PullRequest>(`/api/v1/runs/${runId}/pull-requests`, body),
  push: (runId: string) =>
    postJson<{ headSha?: string; pushUrl?: string; operationId?: string }>(
      `/api/v1/runs/${runId}/git/push`,
    ),
};
