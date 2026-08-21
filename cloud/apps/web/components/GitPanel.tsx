"use client";

import { useCallback, useState } from "react";
import type { PullRequest, PullRequestState } from "@/lib/types";
import { readSessionUi, writeSessionUi, type SessionGitSubTab } from "@/lib/sessionUi";
import { markPullRequestReady, mergePullRequest, isDraftPullRequest } from "@/lib/gitPublish";
import { ChangesPanel } from "./ChangesPanel";
import { CommitsPanel } from "./CommitsPanel";
import { ReviewPanel } from "./ReviewPanel";
import { IconExternal } from "@/lib/icons";
import { useToast } from "./Toast";

export type GitSubTab = SessionGitSubTab;

function prStateClass(state?: PullRequestState | string): string {
  const s = (state || "").toLowerCase();
  if (s === "merged") return "bg-active text-ink";
  if (s === "open") return "bg-ok-soft text-ok";
  if (s === "draft") return "bg-tertiary text-muted";
  if (s === "closed") return "bg-danger-soft text-danger";
  return "bg-tertiary text-ink";
}

function prStateLabel(state?: PullRequestState | string): string {
  const s = (state || "").toLowerCase();
  if (s === "open") return "Open";
  if (s === "draft") return "Draft";
  if (s === "merged") return "Merged";
  if (s === "closed") return "Closed";
  return state || "";
}

export function GitPanel({
  runId,
  defaultTitle,
  defaultBaseRef,
  headBranch,
  pullRequest,
  onPullRequestChange,
  onCommitAndCreatePr,
  commitBusy,
  refreshSignal,
  onRefresh,
}: {
  runId: string;
  defaultTitle?: string;
  defaultBaseRef?: string;
  headBranch?: string;
  pullRequest?: PullRequest | null;
  onPullRequestChange?: (pr: PullRequest | null) => void;
  onCommitAndCreatePr?: () => void | Promise<void>;
  commitBusy?: boolean;
  refreshSignal?: number;
  onRefresh?: () => void;
}) {
  const toast = useToast();
  const [subTab, setSubTabState] = useState<GitSubTab>(() => {
    const saved = readSessionUi(runId).gitSubTab;
    return saved === "review" || saved === "commits" || saved === "diff" ? saved : "diff";
  });
  const [actionBusy, setActionBusy] = useState(false);
  const [localRefreshSignal, setLocalRefreshSignal] = useState(0);
  const setSubTab = useCallback(
    (next: GitSubTab) => {
      setSubTabState(next);
      writeSessionUi(runId, { gitSubTab: next });
    },
    [runId],
  );
  const title = pullRequest?.title || defaultTitle || "Changes";
  const base = defaultBaseRef || "main";
  const prState = pullRequest?.state;
  const prUrl = pullRequest?.url;
  const prId = pullRequest?.id;

  const triggerRefresh = useCallback(() => {
    setLocalRefreshSignal((n) => n + 1);
    onRefresh?.();
  }, [onRefresh]);

  const markReady = useCallback(async () => {
    if (!prId) return;
    setActionBusy(true);
    try {
      const updated = await markPullRequestReady(runId, prId);
      onPullRequestChange?.(updated);
      triggerRefresh();
      toast("Pull request marked as ready", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setActionBusy(false);
    }
  }, [prId, runId, onPullRequestChange, triggerRefresh, toast]);

  const merge = useCallback(async () => {
    if (!prId) return;
    setActionBusy(true);
    try {
      const updated = await mergePullRequest(runId, prId);
      onPullRequestChange?.(updated);
      triggerRefresh();
      toast("Pull request merged", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setActionBusy(false);
    }
  }, [prId, runId, onPullRequestChange, triggerRefresh, toast]);

  const isDraft = pullRequest ? isDraftPullRequest(pullRequest) : false;
  const isMerged = prState === "merged";
  const isClosed = prState === "closed";

  const showMarkReady = isDraft && !!prId;
  const showMerge = !!prId && !isDraft && !isMerged && !isClosed;
  const showCommit = !pullRequest && onCommitAndCreatePr;

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_auto_minmax(0,1fr)]">
      <div className="flex items-start justify-between gap-2 bg-canvas px-3 py-2.5">
        <div className="min-w-0 flex-1">
          {prUrl ? (
            <a
              href={prUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex max-w-full items-center gap-1 truncate text-[13px] font-semibold leading-snug text-ink hover:underline"
              title={title}
            >
              <span className="truncate">{title}</span>
              {pullRequest?.providerNumber != null ? (
                <span className="shrink-0 text-muted">#{pullRequest.providerNumber}</span>
              ) : null}
              <IconExternal className="h-3.5 w-3.5 shrink-0 text-muted" />
            </a>
          ) : (
            <div className="truncate text-[13px] font-semibold leading-snug text-ink" title={title}>
              {title}
            </div>
          )}
          <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-muted">
            {(prState || isDraft) && (
              <span
                className={`rounded px-1.5 py-0.5 text-[10px] font-semibold ${prStateClass(isDraft ? "draft" : prState)}`}
              >
                {prStateLabel(isDraft ? "draft" : prState)}
              </span>
            )}
            {headBranch ? (
              <span className="font-mono">
                {headBranch} → {base}
              </span>
            ) : (
              <span>vs {base}</span>
            )}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {showMarkReady ? (
            <button
              type="button"
              className="btn btn-primary btn-sm"
              disabled={actionBusy}
              onClick={() => void markReady()}
            >
              {actionBusy ? "Updating…" : "Mark as ready"}
            </button>
          ) : null}
          {showMerge ? (
            <button
              type="button"
              className="btn btn-primary btn-sm"
              disabled={actionBusy}
              onClick={() => void merge()}
            >
              {actionBusy ? "Merging…" : "Merge"}
            </button>
          ) : null}
        </div>
      </div>
      <div className="flex h-8 items-center justify-between gap-2 border-b border-line bg-canvas px-2">
        <div className="flex min-w-0 items-center gap-0.5">
          {(
            [
              { id: "diff" as const, label: "Changes" },
              { id: "review" as const, label: "Review" },
              { id: "commits" as const, label: "Commits" },
            ] as const
          ).map((t) => (
            <button
              key={t.id}
              type="button"
              className={[
                "-mb-px border-b-2 px-2.5 py-1.5 text-[12px] font-medium",
                subTab === t.id
                  ? "border-ink text-ink"
                  : "border-transparent text-muted hover:text-ink",
              ].join(" ")}
              onClick={() => setSubTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        {showCommit ? (
          <button
            type="button"
            className="btn btn-primary btn-sm shrink-0"
            disabled={commitBusy}
            onClick={() => void onCommitAndCreatePr()}
          >
            {commitBusy ? "Committing…" : "Commit & Create PR"}
          </button>
        ) : null}
      </div>
      <div className="min-h-0 overflow-hidden">
        {subTab === "diff" && (
          <ChangesPanel
            runId={runId}
            baseRef={defaultBaseRef}
            refreshSignal={(refreshSignal || 0) + localRefreshSignal}
          />
        )}
        {subTab === "review" && (
          <ReviewPanel
            runId={runId}
            baseRef={defaultBaseRef}
            refreshSignal={(refreshSignal || 0) + localRefreshSignal}
          />
        )}
        {subTab === "commits" && (
          <CommitsPanel
            runId={runId}
            baseRef={defaultBaseRef}
            refreshSignal={(refreshSignal || 0) + localRefreshSignal}
          />
        )}
      </div>
    </div>
  );
}
