"use client";

import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api";
import type { GitCompare, GitStatusFile } from "@/lib/types";
import { IconRefresh } from "@/lib/icons";
import { DiffViewer } from "./DiffViewer";
import { useToast } from "./Toast";

function statusClass(status: string): string {
  if (status === "A" || status === "?") return "text-ok";
  if (status === "D") return "text-danger line-through";
  if (status === "U") return "text-warn-ink";
  return "text-warn-ink";
}

function statusLabel(status: string): string {
  if (status === "?") return "U";
  return status || "M";
}

export function ChangesPanel({
  runId,
  baseRef,
  banner,
}: {
  runId: string;
  baseRef?: string;
  banner?: string;
}) {
  const toast = useToast();
  const [compare, setCompare] = useState<GitCompare | null>(null);
  const [error, setError] = useState("");
  const [selected, setSelected] = useState<string | null>(null);
  const [diff, setDiff] = useState("");
  const [diffError, setDiffError] = useState("");
  const [loadingDiff, setLoadingDiff] = useState(false);

  const baseLabel = compare?.base || baseRef || "main";

  const loadCompare = useCallback(async () => {
    try {
      const data = await api<GitCompare>(`/api/v1/runs/${runId}/git/compare`);
      setCompare(data);
      setError("");
      setSelected((prev) => {
        if (prev && data.files.some((f) => f.path === prev)) return prev;
        return data.files[0]?.path ?? null;
      });
    } catch (err) {
      setCompare(null);
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [runId]);

  const loadFileDiff = useCallback(
    async (path: string) => {
      setLoadingDiff(true);
      setDiffError("");
      try {
        const data = await api<{ diff?: string }>(
          `/api/v1/runs/${runId}/git/compare/diff?path=${encodeURIComponent(path)}`,
        );
        setDiff((data?.diff || "").trim());
      } catch (err) {
        setDiff("");
        setDiffError(err instanceof Error ? err.message : String(err));
      } finally {
        setLoadingDiff(false);
      }
    },
    [runId],
  );

  useEffect(() => {
    loadCompare();
  }, [loadCompare]);

  useEffect(() => {
    if (selected) loadFileDiff(selected);
    else {
      setDiff("");
      setDiffError("");
    }
  }, [selected, loadFileDiff]);

  const files = compare?.files ?? [];
  const totalAdd = compare?.totalAdditions ?? 0;
  const totalDel = compare?.totalDeletions ?? 0;

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_minmax(0,1fr)]">
      {banner && (
        <div className="border-b border-line bg-secondary px-2.5 py-1.5 text-[11px] text-muted">
          {banner}
        </div>
      )}
      <div className="grid min-h-0 grid-cols-[200px_minmax(0,1fr)]">
        <div className="flex min-h-0 flex-col border-r border-line bg-canvas">
          <div className="flex items-center justify-between gap-2 border-b border-line px-2 py-1.5">
            <div className="min-w-0 text-[11px] leading-snug text-muted">
              <span className="font-medium text-ink">{files.length}</span> changes
              {files.length > 0 && (
                <>
                  {" · "}
                  <span className="text-ok">+{totalAdd}</span>{" "}
                  <span className="text-danger">-{totalDel}</span>
                </>
              )}
            </div>
            <button
              type="button"
              className="btn btn-sm !px-1.5"
              title="Refresh"
              aria-label="Refresh changes"
              onClick={() => {
                loadCompare().catch((e) => toast(String(e), "error"));
              }}
            >
              <IconRefresh className="h-3.5 w-3.5" />
            </button>
          </div>
          <div className="min-h-0 flex-1 overflow-auto py-1">
            {error && <div className="px-2.5 py-2 text-[12px] text-danger">{error}</div>}
            {!error && !files.length && (
              <div className="px-2.5 py-3 text-[12px] text-placeholder">
                No changes from {baseLabel}.
              </div>
            )}
            {files.map((f: GitStatusFile) => (
              <button
                key={f.path}
                type="button"
                className={[
                  "flex w-full items-center gap-1.5 px-2 py-1.5 text-left text-[11px] hover:bg-secondary",
                  selected === f.path ? "bg-secondary" : "",
                ].join(" ")}
                onClick={() => setSelected(f.path)}
              >
                <span className={`w-3 shrink-0 font-semibold ${statusClass(f.status)}`}>
                  {statusLabel(f.status)}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-ink" title={f.path}>
                  {f.path}
                </span>
                <span className="shrink-0 font-mono text-[10px] text-muted">
                  <span className="text-ok">+{f.additions}</span>{" "}
                  <span className="text-danger">-{f.deletions}</span>
                </span>
              </button>
            ))}
          </div>
        </div>
        <div className="flex min-h-0 flex-col bg-canvas">
          {selected && (
            <div className="flex items-center justify-between gap-2 border-b border-line bg-canvas px-2.5 py-1.5">
              <code className="truncate font-mono text-[11px] text-muted">{selected}</code>
              {files.find((f) => f.path === selected) && (
                <span className="shrink-0 font-mono text-[11px] text-muted">
                  <span className="text-ok">
                    +{files.find((f) => f.path === selected)!.additions}
                  </span>{" "}
                  <span className="text-danger">
                    -{files.find((f) => f.path === selected)!.deletions}
                  </span>
                </span>
              )}
            </div>
          )}
          <div className="min-h-0 flex-1">
            {diffError ? (
              <div className="p-3 font-mono text-[11px] text-danger">{diffError}</div>
            ) : loadingDiff ? (
              <div className="p-3 text-[12px] text-placeholder">Loading diff…</div>
            ) : (
              <DiffViewer
                diff={diff}
                emptyLabel={selected ? "No diff for this file." : "Select a file."}
              />
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
