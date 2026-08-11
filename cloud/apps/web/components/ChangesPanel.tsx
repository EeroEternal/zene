"use client";

import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "@/lib/api";
import type { GitCompare, GitStatusFile } from "@/lib/types";
import {
  IconChevronDown,
  IconChevronRight,
  IconChevronsCollapse,
  IconChevronsExpand,
  IconRefresh,
} from "@/lib/icons";
import { DiffViewer } from "./DiffViewer";
import { useToast } from "./Toast";

function isAdded(status: string): boolean {
  return status === "A" || status === "?";
}

function isDeleted(status: string): boolean {
  return status === "D";
}

function FileDiffBlock({
  runId,
  file,
  open,
  onToggle,
  autoLoad,
}: {
  runId: string;
  file: GitStatusFile;
  open: boolean;
  onToggle: () => void;
  autoLoad: boolean;
}) {
  const [diff, setDiff] = useState("");
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      const data = await api<{ diff?: string }>(
        `/api/v1/runs/${runId}/git/compare/diff?path=${encodeURIComponent(file.path)}`,
      );
      setDiff((data?.diff || "").trim());
      setLoaded(true);
    } catch (err) {
      setDiff("");
      setError(err instanceof Error ? err.message : String(err));
      setLoaded(true);
    } finally {
      setLoading(false);
    }
  }, [runId, file.path]);

  useEffect(() => {
    if ((open || autoLoad) && !loaded && !loading) {
      load().catch(() => undefined);
    }
  }, [open, autoLoad, loaded, loading, load]);

  return (
    <section className="border-b border-line last:border-b-0">
      <button
        type="button"
        className="sticky top-0 z-[1] flex w-full items-center gap-2 border-b border-line bg-[#f6f8fa] px-3 py-2 text-left hover:bg-secondary"
        onClick={onToggle}
        aria-expanded={open}
      >
        {open ? (
          <IconChevronDown className="h-3.5 w-3.5 shrink-0 text-muted" />
        ) : (
          <IconChevronRight className="h-3.5 w-3.5 shrink-0 text-muted" />
        )}
        <code className="min-w-0 flex-1 truncate font-mono text-[12px] font-medium text-ink" title={file.path}>
          {file.path}
        </code>
        {isAdded(file.status) && (
          <span className="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold text-[#1a7f37] bg-[#e6ffec]">
            New
          </span>
        )}
        {isDeleted(file.status) && (
          <span className="shrink-0 rounded px-1.5 py-0.5 text-[10px] font-semibold text-[#cf222e] bg-[#ffebe9]">
            Deleted
          </span>
        )}
        <span className="shrink-0 font-mono text-[11px] tabular-nums">
          <span className="text-[#1a7f37]">+{file.additions}</span>{" "}
          <span className="text-[#cf222e]">−{file.deletions}</span>
        </span>
      </button>
      {open && (
        <div className="min-h-0 bg-canvas">
          {error ? (
            <div className="p-3 font-mono text-[11px] text-danger">{error}</div>
          ) : loading || !loaded ? (
            <div className="p-3 text-[12px] text-placeholder">Loading diff…</div>
          ) : (
            <DiffViewer diff={diff} emptyLabel="No diff for this file." />
          )}
        </div>
      )}
    </section>
  );
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
  const [openMap, setOpenMap] = useState<Record<string, boolean>>({});

  const baseLabel = compare?.base || baseRef || "main";

  const loadCompare = useCallback(async () => {
    try {
      const data = await api<GitCompare>(`/api/v1/runs/${runId}/git/compare`);
      setCompare(data);
      setError("");
      setOpenMap((prev) => {
        const next: Record<string, boolean> = {};
        data.files.forEach((f, idx) => {
          next[f.path] = prev[f.path] ?? idx < 12;
        });
        return next;
      });
    } catch (err) {
      setCompare(null);
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [runId]);

  useEffect(() => {
    loadCompare();
  }, [loadCompare]);

  const files = compare?.files ?? [];
  const totalAdd = compare?.totalAdditions ?? 0;
  const totalDel = compare?.totalDeletions ?? 0;

  const expandAll = useCallback(() => {
    setOpenMap(Object.fromEntries(files.map((f) => [f.path, true])));
  }, [files]);

  const collapseAll = useCallback(() => {
    setOpenMap(Object.fromEntries(files.map((f) => [f.path, false])));
  }, [files]);

  const allOpen = useMemo(
    () => files.length > 0 && files.every((f) => openMap[f.path]),
    [files, openMap],
  );

  return (
    <div className="grid h-full min-h-0 grid-rows-[auto_minmax(0,1fr)] bg-canvas">
      {banner && (
        <div className="border-b border-line bg-secondary px-3 py-1.5 text-[11px] text-muted">
          {banner}
        </div>
      )}
      <div className="flex min-h-0 flex-col">
        <div className="flex h-8 shrink-0 items-center justify-between gap-2 border-b border-line px-3">
          <div className="min-w-0 font-mono text-[11px] text-muted">
            <span className="font-medium text-ink">{files.length}</span>
            {" files"}
            {files.length > 0 && (
              <>
                {" · "}
                <span className="text-[#1a7f37]">+{totalAdd}</span>{" "}
                <span className="text-[#cf222e]">−{totalDel}</span>
                {" vs "}
                <span className="text-ink">{baseLabel}</span>
              </>
            )}
          </div>
          <div className="flex shrink-0 items-center gap-0.5">
            {files.length > 0 && (
              <button
                type="button"
                className="inline-flex h-6 items-center gap-1 rounded-md px-1.5 text-[11px] font-medium text-muted hover:bg-secondary hover:text-ink"
                onClick={() => (allOpen ? collapseAll() : expandAll())}
              >
                {allOpen ? (
                  <IconChevronsCollapse className="h-3.5 w-3.5" />
                ) : (
                  <IconChevronsExpand className="h-3.5 w-3.5" />
                )}
                {allOpen ? "Collapse" : "Expand"}
              </button>
            )}
            <button
              type="button"
              className="inline-flex h-6 w-6 items-center justify-center rounded-md text-muted hover:bg-secondary hover:text-ink"
              title="Refresh"
              aria-label="Refresh changes"
              onClick={() => {
                loadCompare().catch((e) => toast(String(e), "error"));
              }}
            >
              <IconRefresh className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        <div className="min-h-0 flex-1 overflow-auto">
          {error && <div className="px-3 py-3 text-[12px] text-danger">{error}</div>}
          {!error && !files.length && (
            <div className="flex h-full min-h-[160px] flex-col items-center justify-center gap-1 px-4 text-center">
              <div className="text-[13px] font-medium text-ink">No changes from {baseLabel}</div>
              <p className="m-0 max-w-[260px] text-[12px] leading-snug text-placeholder">
                File diffs appear here after the agent edits the workspace.
              </p>
            </div>
          )}
          {files.map((f, idx) => (
            <FileDiffBlock
              key={f.path}
              runId={runId}
              file={f}
              open={!!openMap[f.path]}
              autoLoad={idx < 4}
              onToggle={() => {
                setOpenMap((prev) => ({ ...prev, [f.path]: !prev[f.path] }));
              }}
            />
          ))}
        </div>
      </div>
    </div>
  );
}
