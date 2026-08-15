"use client";

import { useCallback, useState } from "react";
import type { PullRequestState } from "@/lib/types";
import { readSessionUi, writeSessionUi, type SessionGitSubTab } from "@/lib/sessionUi";
import { ChangesPanel } from "./ChangesPanel";
import { CommitsPanel } from "./CommitsPanel";
import { ReviewPanel } from "./ReviewPanel";
import { IconUpload } from "@/lib/icons";

export type GitSubTab = SessionGitSubTab;

function prStateClass(state?: PullRequestState | string): string {
  const s = (state || "").toLowerCase();
  if (s === "merged") return "bg-active text-ink";
  if (s === "open") return "bg-ok-soft text-ok";
  if (s === "draft") return "bg-tertiary text-muted";
  if (s === "closed") return "bg-danger-soft text-danger";
  return "bg-tertiary text-ink";
}

export function GitPanel({
  runId,
  defaultTitle,
  defaultBaseRef,
  headBranch,
  prUrl,
  prState,
  onPush,
  pushBusy,
  onCreatePr,
}: {
  runId: string;
  defaultTitle?: string;
  defaultBaseRef?: string;
  headBranch?: string;
  prUrl?: string;
  prState?: PullRequestState;
  onPush?: () => void | Promise<void>;
  pushBusy?: boolean;
  onCreatePr?: () => void;
}) {
  const [subTab, setSubTabState] = useState<GitSubTab>(() => {
    const saved = readSessionUi(runId).gitSubTab;
    return saved === "review" || saved === "commits" || saved === "diff" ? saved : "diff";
  });
  const setSubTab = useCallback(
    (next: GitSubTab) => {
      setSubTabState(next);
      writeSessionUi(runId, { gitSubTab: next });
    },
    [runId],
  );
  const title = defaultTitle || "Changes";
  const base = defaultBaseRef || "main";

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_auto_minmax(0,1fr)]">
      <div className="bg-canvas px-3 py-2.5">
        <div className="min-w-0">
          {prUrl ? (
            <a
              href={prUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="block truncate text-[13px] font-semibold leading-snug text-ink hover:underline"
              title={title}
            >
              {title}
            </a>
          ) : (
            <div className="truncate text-[13px] font-semibold leading-snug text-ink" title={title}>
              {title}
            </div>
          )}
          <div className="mt-1 flex flex-wrap items-center gap-1.5 text-[11px] text-muted">
            {prState && (
              <span
                className={`rounded px-1.5 py-0.5 text-[10px] font-semibold capitalize ${prStateClass(prState)}`}
              >
                {prState}
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
      </div>
      <div className="flex h-8 items-center justify-between gap-2 border-b border-line bg-canvas px-2">
        <div className="flex min-w-0 items-center gap-0.5">
          {(
            [
              { id: "diff" as const, label: "Diff" },
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
        <div className="flex shrink-0 items-center gap-1">
          {onCreatePr ? (
            <button type="button" className="btn btn-sm hidden min-[641px]:inline-flex" onClick={onCreatePr}>
              Create PR
            </button>
          ) : null}
          {onPush ? (
            <button
              type="button"
              className="btn btn-primary btn-sm inline-flex items-center gap-1"
              disabled={pushBusy}
              onClick={() => void onPush()}
            >
              <IconUpload className="h-3.5 w-3.5" />
              {pushBusy ? "Pushing…" : "Push"}
            </button>
          ) : null}
        </div>
      </div>
      <div className="min-h-0 overflow-hidden">
        {subTab === "diff" && <ChangesPanel runId={runId} baseRef={defaultBaseRef} />}
        {subTab === "review" && <ReviewPanel runId={runId} baseRef={defaultBaseRef} />}
        {subTab === "commits" && <CommitsPanel runId={runId} baseRef={defaultBaseRef} />}
      </div>
    </div>
  );
}
