import { api } from "@/lib/api";
import { runsApi } from "@/lib/cloud";
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

function isDraftPullRequest(pr: PullRequest): boolean {
  if (pr.draft) return true;
  return (pr.state || "").toLowerCase() === "draft";
}

function isActivePullRequest(pr: PullRequest): boolean {
  if (!pr.url) return false;
  const state = (pr.state || "").toLowerCase();
  return state === "open" || state === "draft";
}

export { isDraftPullRequest, isActivePullRequest };

export interface PublishResult {
  push: PushResult;
  pullRequest?: PullRequest;
}

export async function fetchGitCompare(runId: string): Promise<GitCompare | null> {
  try {
    return await runsApi.gitCompare(runId);
  } catch {
    return null;
  }
}

export async function fetchRunPullRequests(runId: string): Promise<PullRequest[]> {
  try {
    return (await runsApi.pullRequests(runId)) || [];
  } catch {
    return [];
  }
}

export async function pushRunBranch(runId: string): Promise<PushResult> {
  return runsApi.push(runId);
}

export async function createRunPullRequest(
  runId: string,
  opts: PublishOptions,
): Promise<PullRequest> {
  return runsApi.createPullRequest(runId, {
    title: opts.title.trim(),
    body: opts.body?.trim() || undefined,
    draft: opts.draft ?? true,
    baseRef: opts.baseRef?.trim() || undefined,
    headRef: opts.headBranch?.trim() || undefined,
  });
}

/** Commit workspace changes, push branch, then open a draft PR when none exists yet. */
export async function commitAndCreatePullRequest(
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

/** @deprecated Use commitAndCreatePullRequest */
export const publishRunToGithub = commitAndCreatePullRequest;

export async function markPullRequestReady(
  runId: string,
  prId: string,
): Promise<PullRequest> {
  return api<PullRequest>(`/api/v1/runs/${runId}/pull-requests/${prId}/ready`, {
    method: "POST",
    body: "{}",
  });
}

export async function mergePullRequest(
  runId: string,
  prId: string,
): Promise<PullRequest> {
  return api<PullRequest>(`/api/v1/runs/${runId}/pull-requests/${prId}/merge`, {
    method: "POST",
    body: "{}",
  });
}

export function hasUnpublishedChanges(
  compare: GitCompare | null | undefined,
  pullRequests: PullRequest[],
): boolean {
  if (!compare?.files?.length) return false;
  return !pullRequests.some(isActivePullRequest);
}
