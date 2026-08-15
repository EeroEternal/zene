"use client";

import { useCallback, useState } from "react";
import type { PullRequest } from "@/lib/types";
import {
  isDraftPullRequest,
  markPullRequestReady,
  mergePullRequest,
} from "@/lib/gitPublish";
import { IconExternal, IconGithub } from "@/lib/icons";
import { useToast } from "./Toast";

export function PullRequestCard({
  runId,
  pullRequest,
  onUpdated,
}: {
  runId: string;
  pullRequest: PullRequest;
  onUpdated?: (pr: PullRequest) => void;
}) {
  const toast = useToast();
  const [busy, setBusy] = useState(false);
  const [pr, setPr] = useState(pullRequest);
  const isDraft = isDraftPullRequest(pr);
  const isMerged = (pr.state || "").toLowerCase() === "merged";

  const update = useCallback(
    (next: PullRequest) => {
      setPr(next);
      onUpdated?.(next);
    },
    [onUpdated],
  );

  const markReady = useCallback(async () => {
    if (!pr.id) return;
    setBusy(true);
    try {
      const updated = await markPullRequestReady(runId, pr.id);
      update(updated);
      toast("Pull request marked as ready", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setBusy(false);
    }
  }, [pr.id, runId, update, toast]);

  const merge = useCallback(async () => {
    if (!pr.id) return;
    setBusy(true);
    try {
      const updated = await mergePullRequest(runId, pr.id);
      update(updated);
      toast("Pull request merged", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setBusy(false);
    }
  }, [pr.id, runId, update, toast]);

  return (
    <div className="self-stretch rounded-md border border-line bg-canvas p-3.5 min-[981px]:ml-9">
      <div className="mb-2 flex flex-wrap items-start justify-between gap-2">
        <div className="min-w-0">
          <div className="mb-1 flex items-center gap-2 text-[13px] font-semibold text-ink">
            <IconGithub className="h-4 w-4 shrink-0" />
            {pr.url ? (
              <a
                href={pr.url}
                target="_blank"
                rel="noopener noreferrer"
                className="inline-flex min-w-0 items-center gap-1 hover:underline"
              >
                <span className="truncate">{pr.title}</span>
                {pr.providerNumber != null ? (
                  <span className="shrink-0 text-muted">#{pr.providerNumber}</span>
                ) : null}
                <IconExternal className="h-3.5 w-3.5 shrink-0 text-muted" />
              </a>
            ) : (
              <span className="truncate">{pr.title}</span>
            )}
          </div>
          <p className="m-0 text-[12px] leading-snug text-muted">
            {isMerged
              ? "This pull request has been merged."
              : isDraft
                ? "Draft pull request created. Mark as ready when you want review, then merge."
                : "Pull request is ready for review. Merge when checks pass."}
          </p>
        </div>
        <div className="flex shrink-0 items-center gap-1.5">
          {isDraft ? (
            <button
              type="button"
              className="btn btn-primary btn-sm"
              disabled={busy}
              onClick={() => void markReady()}
            >
              {busy ? "Updating…" : "Mark as ready"}
            </button>
          ) : null}
          {!isDraft && !isMerged ? (
            <button
              type="button"
              className="btn btn-primary btn-sm"
              disabled={busy}
              onClick={() => void merge()}
            >
              {busy ? "Merging…" : "Merge"}
            </button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
