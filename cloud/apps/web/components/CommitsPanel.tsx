"use client";

import { useCallback, useEffect, useState } from "react";
import { runsApi } from "@/lib/cloud";
import type { GitCommit } from "@/lib/types";
import { IconRefresh } from "@/lib/icons";
import { useToast } from "./Toast";

function formatWhen(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return d.toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

export function CommitsPanel({ runId, baseRef }: { runId: string; baseRef?: string }) {
  const toast = useToast();
  const [commits, setCommits] = useState<GitCommit[]>([]);
  const [error, setError] = useState("");
  const baseLabel = baseRef || "main";

  const load = useCallback(async () => {
    try {
      const data = await runsApi.gitCommits(runId);
      setCommits(data || []);
      setError("");
    } catch (err) {
      setCommits([]);
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [runId]);

  useEffect(() => {
    load();
  }, [load]);

  return (
    <div className="flex h-full min-h-0 flex-col bg-canvas">
      <div className="flex items-center justify-between gap-2 border-b border-line px-2 py-1.5">
        <div className="min-w-0 text-[11px] text-muted">
          <span className="font-medium text-ink">{commits.length}</span> commits
          <span className="text-placeholder"> ahead of {baseLabel}</span>
        </div>
        <button
          type="button"
          className="btn btn-sm !px-1.5"
          title="Refresh"
          aria-label="Refresh commits"
          onClick={() => {
            load().catch((e) => toast(String(e), "error"));
          }}
        >
          <IconRefresh className="h-3.5 w-3.5" />
        </button>
      </div>
      <div className="min-h-0 flex-1 overflow-auto py-1">
        {error && <div className="px-2.5 py-2 text-[12px] text-danger">{error}</div>}
        {!error && !commits.length && (
          <div className="px-2.5 py-3 text-[12px] text-placeholder">
            No commits ahead of {baseLabel}.
          </div>
        )}
        {commits.map((c) => (
          <div
            key={c.sha}
            className="border-b border-line px-2.5 py-2 last:border-b-0"
          >
            <div className="truncate text-[12px] font-medium text-ink" title={c.subject}>
              {c.subject || "(no subject)"}
            </div>
            <div className="mt-0.5 flex flex-wrap items-center gap-x-2 gap-y-0.5 font-mono text-[10px] text-muted">
              <span className="text-ink">{c.shortSha || c.sha.slice(0, 7)}</span>
              <span>{c.author}</span>
              <span>{formatWhen(c.authoredAt)}</span>
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
