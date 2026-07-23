"use client";

import { useState } from "react";
import { ChangesPanel } from "./ChangesPanel";
import { CommitsPanel } from "./CommitsPanel";
import { ReviewPanel } from "./ReviewPanel";

export type GitSubTab = "diff" | "review" | "commits";

export function GitPanel({
  runId,
  defaultTitle,
  defaultBaseRef,
  headBranch,
  prUrl,
  prState,
}: {
  runId: string;
  defaultTitle?: string;
  defaultBaseRef?: string;
  headBranch?: string;
  prUrl?: string;
  prState?: string;
}) {
  const [subTab, setSubTab] = useState<GitSubTab>("diff");
  const title = defaultTitle || "Changes";
  const base = defaultBaseRef || "main";

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_auto_minmax(0,1fr)]">
      <div className="border-b border-line bg-canvas px-3 py-2">
        <div className="flex items-start justify-between gap-2">
          <div className="min-w-0">
            <div className="truncate text-[13px] font-semibold text-ink">{title}</div>
            <div className="mt-0.5 flex flex-wrap items-center gap-1.5 text-[11px] text-muted">
              {prState && (
                <span className="rounded bg-secondary px-1.5 py-0.5 font-medium capitalize text-ink">
                  {prState}
                </span>
              )}
              {headBranch && (
                <span className="font-mono">
                  {headBranch} → {base}
                </span>
              )}
            </div>
          </div>
          {prUrl && (
            <a
              href={prUrl}
              target="_blank"
              rel="noopener noreferrer"
              className="btn btn-sm shrink-0"
            >
              View PR
            </a>
          )}
        </div>
      </div>
      <div className="flex h-8 items-center gap-1 border-b border-line bg-canvas px-2">
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
              "rounded-md px-2.5 py-1 text-[12px] font-medium",
              subTab === t.id ? "bg-ink text-white" : "text-muted hover:bg-secondary hover:text-ink",
            ].join(" ")}
            onClick={() => setSubTab(t.id)}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div className="min-h-0 overflow-hidden">
        {subTab === "diff" && <ChangesPanel runId={runId} baseRef={defaultBaseRef} />}
        {subTab === "review" && <ReviewPanel runId={runId} baseRef={defaultBaseRef} />}
        {subTab === "commits" && <CommitsPanel runId={runId} baseRef={defaultBaseRef} />}
      </div>
    </div>
  );
}
