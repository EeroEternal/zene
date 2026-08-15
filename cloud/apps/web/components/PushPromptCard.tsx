"use client";

import { useCallback, useState } from "react";
import type { GitCompare, PullRequest } from "@/lib/types";
import { publishRunToGithub, type PublishResult } from "@/lib/gitPublish";
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
      const next = await publishRunToGithub(runId, {
        title: title?.trim() || "Changes from Zene Cloud",
        baseRef: baseRef || base,
        headBranch: branch,
        draft: true,
      });
      setResult(next);
      onPublished?.(next);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  }, [runId, title, baseRef, base, branch, onPublished]);

  if (result) {
    const pr = result.pullRequest;
    return (
      <div className="self-stretch rounded-md border border-line bg-canvas p-3.5 min-[981px]:ml-9">
        <div className="mb-1 flex items-center gap-2 text-[13px] font-semibold text-ink">
          <IconGithub className="h-4 w-4 shrink-0" />
          <span>已推送到 GitHub</span>
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
              Draft PR{" "}
              <a
                href={pr.url}
                target="_blank"
                rel="noopener noreferrer"
                className="text-ink underline underline-offset-2 hover:text-muted"
              >
                {pr.title}
              </a>{" "}
              已创建。
            </>
          ) : (
            <>分支已推送{result.push.headSha ? ` · ${result.push.headSha.slice(0, 7)}` : ""}。</>
          )}
        </p>
      </div>
    );
  }

  return (
    <div className="self-stretch rounded-md border-l-2 border-primary bg-secondary p-3.5 min-[981px]:ml-9">
      <h4 className="mb-1 text-[13px] font-semibold text-ink">推送到 GitHub？</h4>
      <p className="mb-3 text-[12px] leading-snug text-muted">
        有{" "}
        <span className="font-medium text-ink">
          {fileCount} 个文件
        </span>{" "}
        尚未同步到 GitHub（
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
        ）。推送后将自动创建 Draft PR。
      </p>
      <div className="flex flex-wrap gap-2">
        <button type="button" className="btn btn-primary btn-sm" disabled={busy} onClick={() => void publish()}>
          {busy ? "推送中…" : "推送并创建 PR"}
        </button>
        <button type="button" className="btn btn-sm" disabled={busy} onClick={onDismiss}>
          暂不推送
        </button>
      </div>
      {error ? <div className="mt-2 text-[12px] text-danger">{error}</div> : null}
    </div>
  );
}
