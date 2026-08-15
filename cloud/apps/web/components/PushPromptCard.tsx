"use client";

import { useCallback, useState } from "react";
import type { GitCompare } from "@/lib/types";
import { commitAndCreatePullRequest, type PublishResult } from "@/lib/gitPublish";
import { IconExternal, IconGithub } from "@/lib/icons";

export function PushPromptCard({
  runId,
  title,
  baseRef,
  headBranch,
  compare,
  onPublished,
  onDismiss,
}: {
  runId: string;
  title?: string;
  baseRef?: string;
  headBranch?: string;
  compare: GitCompare;
  onPublished?: (result: PublishResult) => void;
  onDismiss?: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [result, setResult] = useState<PublishResult | null>(null);

  const fileCount = compare.files.length;
  const additions = compare.totalAdditions;
  const deletions = compare.totalDeletions;
  const base = compare.base || baseRef || "main";
  const branch = headBranch || compare.head;

  const publish = useCallback(async () => {
    setError("");
    setBusy(true);
    try {
      const next = await commitAndCreatePullRequest(runId, {
        title: title?.trim() || "Changes from Zene Cloud",
        baseRef: baseRef || base,
        headBranch: branch,
        compare,
        draft: true,
      });
      setResult(next);
      onPublished?.(next);
      if (next.pullRequest?.url) {
        window.open(next.pullRequest.url, "_blank", "noopener,noreferrer");
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }, [runId, title, baseRef, base, branch, compare, onPublished]);

  if (result) {
    const pr = result.pullRequest;
    return (
      <div className="self-stretch rounded-md border border-line bg-canvas p-3.5 min-[981px]:ml-9">
        <div className="mb-1 flex items-center gap-2 text-[13px] font-semibold text-ink">
          <IconGithub className="h-4 w-4 shrink-0" />
          <span>Draft PR created</span>
          {pr?.providerNumber != null && pr.url ? (
            <a
              href={pr.url}
              target="_blank"
              rel="noopener noreferrer"
              className="inline-flex items-center gap-1 text-primary hover:underline"
            >
              #{pr.providerNumber}
              <IconExternal className="h-3.5 w-3.5" />
            </a>
          ) : null}
        </div>
        <p className="m-0 text-[12px] leading-snug text-muted">
          {pr?.url ? (
            <>
              Open{" "}
              <a
                href={pr.url}
                target="_blank"
                rel="noopener noreferrer"
                className="text-ink underline underline-offset-2 hover:text-muted"
              >
                {pr.title}
              </a>{" "}
              to mark as ready, then merge when checks pass.
            </>
          ) : (
            <>Changes committed{result.push.headSha ? ` · ${result.push.headSha.slice(0, 7)}` : ""}.</>
          )}
        </p>
      </div>
    );
  }

  return (
    <div className="self-stretch rounded-md border-l-2 border-primary bg-secondary p-3.5 min-[981px]:ml-9">
      <h4 className="mb-1 text-[13px] font-semibold text-ink">Commit & Create PR?</h4>
      <p className="mb-3 text-[12px] leading-snug text-muted">
        <span className="font-medium text-ink">{fileCount} file(s)</span> changed (
        <span className="font-mono text-[#1a7f37]">+{additions}</span>{" "}
        <span className="font-mono text-[#cf222e]">−{deletions}</span>
        {" vs "}
        {base}
        {branch ? (
          <>
            {" · "}
            <span className="font-mono text-ink">{branch}</span>
          </>
        ) : null}
        ). Changes will be committed, pushed, and opened as a draft PR.
      </p>
      <div className="flex flex-wrap gap-2">
        <button type="button" className="btn btn-primary btn-sm" disabled={busy} onClick={() => void publish()}>
          {busy ? "Committing…" : "Commit & Create PR"}
        </button>
        <button type="button" className="btn btn-sm" disabled={busy} onClick={onDismiss}>
          Not now
        </button>
      </div>
      {error ? <div className="mt-2 text-[12px] text-danger">{error}</div> : null}
    </div>
  );
}
