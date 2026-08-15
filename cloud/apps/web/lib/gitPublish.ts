import { runsApi } from "@/lib/cloud";
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
}

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

/** Push branch to GitHub, then open a draft PR when none exists yet. */
export async function publishRunToGithub(
  runId: string,
  opts: PublishOptions,
): Promise<PublishResult> {
  const push = await pushRunBranch(runId);
  const existing = await fetchRunPullRequests(runId);
  const linked = existing.find((pr) => pr.url);
  if (linked) {
    return { push, pullRequest: linked };
  }
  const pullRequest = await createRunPullRequest(runId, opts);
  return { push, pullRequest };
}

export function hasUnpublishedChanges(
  compare: GitCompare | null | undefined,
  pullRequests: PullRequest[],
): boolean {
  if (!compare?.files?.length) return false;
  return !pullRequests.some((pr) => Boolean(pr.url));
}
