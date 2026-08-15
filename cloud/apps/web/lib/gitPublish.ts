import { api } from "@/lib/api";
import { buildDefaultPrBody } from "@/lib/prBody";
import type { GitCompare, PullRequest } from "@/lib/types";

export interface PushResult {
  headSha?: string;
  pushUrl?: string;
  operationId?: string;
}

export interface PublishOptions {
  title: string;
  baseRef?: string;
  headBranch?: string;
  body?: string;
  draft?: boolean;
  compare?: GitCompare | null;
}

function isActivePullRequest(pr: PullRequest): boolean {
  if (!pr.url) return false;
  const state = (pr.state || "").toLowerCase();
  return state === "open" || state === "draft";
}

export interface PublishResult {
  push: PushResult;
  pullRequest?: PullRequest;
}

export async function fetchGitCompare(runId: string): Promise<GitCompare | null> {
  try {
    return await api<GitCompare>(`/api/v1/runs/${runId}/git/compare`);
  } catch {
    return null;
  }
}

export async function fetchRunPullRequests(runId: string): Promise<PullRequest[]> {
  try {
    return (await api<PullRequest[]>(`/api/v1/runs/${runId}/pull-requests`)) || [];
  } catch {
    return [];
  }
}

export async function pushRunBranch(runId: string): Promise<PushResult> {
  return api<PushResult>(`/api/v1/runs/${runId}/git/push`, {
    method: "POST",
    body: "{}",
  });
}

export async function createRunPullRequest(
  runId: string,
  opts: PublishOptions,
): Promise<PullRequest> {
  return api<PullRequest>(`/api/v1/runs/${runId}/pull-requests`, {
    method: "POST",
    body: JSON.stringify({
      title: opts.title.trim(),
      body: opts.body?.trim() || undefined,
      draft: opts.draft ?? true,
      baseRef: opts.baseRef?.trim() || undefined,
      headRef: opts.headBranch?.trim() || undefined,
    }),
  });
}

/** Push branch to GitHub, then open a draft PR when none exists yet. */
export async function publishRunToGithub(
  runId: string,
  opts: PublishOptions,
): Promise<PublishResult> {
  const push = await pushRunBranch(runId);
  const existing = await fetchRunPullRequests(runId);
  const linked = existing.find(isActivePullRequest);
  if (linked) {
    return { push, pullRequest: linked };
  }
  const body = opts.body?.trim() || buildDefaultPrBody(opts.compare);
  const pullRequest = await createRunPullRequest(runId, { ...opts, body });
  return { push, pullRequest };
}

export function hasUnpublishedChanges(
  compare: GitCompare | null | undefined,
  pullRequests: PullRequest[],
): boolean {
  if (!compare?.files?.length) return false;
  return !pullRequests.some(isActivePullRequest);
}
