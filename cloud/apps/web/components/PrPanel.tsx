"use client";

import { useCallback, useEffect, useState } from "react";
import { api } from "@/lib/api";
import type { PullRequest } from "@/lib/types";
import { IconExternal, IconRefresh } from "@/lib/icons";
import { useToast } from "./Toast";

export function PrPanel({
  runId,
  defaultTitle,
  defaultBaseRef,
  headBranch,
  compact,
}: {
  runId: string;
  defaultTitle?: string;
  defaultBaseRef?: string;
  headBranch?: string;
  compact?: boolean;
}) {
  const toast = useToast();
  const [prs, setPrs] = useState<PullRequest[]>([]);
  const [error, setError] = useState("");
  const [prBusy, setPrBusy] = useState(false);
  const [pushBusy, setPushBusy] = useState(false);

  const [title, setTitle] = useState(defaultTitle || "");
  const [body, setBody] = useState("");
  const [draft, setDraft] = useState(true);
  const [baseRef, setBaseRef] = useState(defaultBaseRef || "");

  useEffect(() => {
    if (defaultTitle) setTitle((t) => t || defaultTitle);
  }, [defaultTitle]);

  useEffect(() => {
    if (defaultBaseRef) setBaseRef((b) => b || defaultBaseRef);
  }, [defaultBaseRef]);

  const loadPrs = useCallback(async () => {
    setError("");
    try {
      setPrs((await api<PullRequest[]>(`/api/v1/runs/${runId}/pull-requests`)) || []);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [runId]);

  useEffect(() => {
    loadPrs();
  }, [loadPrs]);

  const pushBranch = useCallback(async () => {
    setError("");
    setPushBusy(true);
    try {
      const result = await api<{ headSha?: string; pushUrl?: string }>(`/api/v1/runs/${runId}/git/push`, {
        method: "POST",
        body: "{}",
      });
      toast(`Pushed · ${result.headSha || result.pushUrl || "ok"}`, "ok");
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      toast(msg, "error");
    } finally {
      setPushBusy(false);
    }
  }, [runId, toast]);

  const createPr = useCallback(async () => {
    const t = title.trim();
    if (!t) {
      toast("Title is required", "error");
      return;
    }
    setError("");
    setPrBusy(true);
    try {
      await api(`/api/v1/runs/${runId}/pull-requests`, {
        method: "POST",
        body: JSON.stringify({
          title: t,
          body: body.trim() || undefined,
          draft,
          baseRef: baseRef.trim() || undefined,
          headRef: headBranch || undefined,
        }),
      });
      toast("Pull request created", "ok");
      await loadPrs();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      toast(msg, "error");
    } finally {
      setPrBusy(false);
    }
  }, [runId, title, body, draft, baseRef, headBranch, loadPrs, toast]);

  return (
    <div className={`flex h-full min-h-0 flex-col overflow-auto ${compact ? "px-3 pb-3 pt-2" : "px-4 pb-5 pt-3.5"}`}>
      {!compact && <h3 className="panel-title">Create pull request</h3>}
      {headBranch && (
        <div className={`font-mono text-[11px] text-muted ${compact ? "mb-2" : "mb-3"}`}>
          {headBranch} → {baseRef || "main"}
        </div>
      )}
      <label className="mb-1 block text-[11px] font-medium uppercase tracking-wider text-muted">Title</label>
      <input
        className="field-input mb-3"
        value={title}
        onChange={(e) => setTitle(e.target.value)}
        placeholder="PR title"
      />
      <label className="mb-1 block text-[11px] font-medium uppercase tracking-wider text-muted">Body</label>
      <textarea
        className="field-input mb-3 min-h-[88px] resize-y"
        value={body}
        onChange={(e) => setBody(e.target.value)}
        placeholder="Describe the changes…"
      />
      <label className="mb-1 block text-[11px] font-medium uppercase tracking-wider text-muted">Base</label>
      <input
        className="field-input mb-3 font-mono"
        value={baseRef}
        onChange={(e) => setBaseRef(e.target.value)}
        placeholder="main"
      />
      <label className="mb-4 flex items-center gap-2 text-[13px] text-ink">
        <input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} />
        Create as draft
      </label>
      <div className="mb-4 flex flex-wrap gap-2">
        <button type="button" className="btn btn-primary btn-sm" disabled={prBusy} onClick={createPr}>
          Create PR
        </button>
        <button type="button" className="btn btn-sm" disabled={pushBusy} onClick={pushBranch}>
          Push
        </button>
        <button type="button" className="btn btn-sm" onClick={loadPrs} title="Refresh" aria-label="Refresh PRs">
          <IconRefresh className="h-3.5 w-3.5" />
        </button>
      </div>

      {!compact && <h3 className="panel-title">Pull requests</h3>}
      {compact && <div className="mb-2 text-[11px] font-medium uppercase tracking-wider text-muted">Pull requests</div>}
      <div>
        {prs.map((pr, i) => (
          <div key={i} className="mb-2.5 rounded-lg border border-line bg-canvas p-3">
            <div className="mb-1 flex items-start justify-between gap-2 text-[13px] font-semibold">
              {pr.url ? (
                <a
                  className="text-ink underline underline-offset-2 hover:text-muted"
                  href={pr.url}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {pr.title}
                </a>
              ) : (
                <span>{pr.title}</span>
              )}
              {pr.url && (
                <a
                  href={pr.url}
                  target="_blank"
                  rel="noopener noreferrer"
                  className="shrink-0 text-muted hover:text-ink"
                  aria-label="Open on GitHub"
                  title="Open on GitHub"
                >
                  <IconExternal className="h-3.5 w-3.5" />
                </a>
              )}
            </div>
            <div className="font-mono text-[11px] text-muted">
              #{pr.providerNumber ?? "—"} · {pr.state}
              {pr.draft ? " · draft" : ""}
            </div>
          </div>
        ))}
      </div>
      {!prs.length && !error && (
        <div className="py-3 text-[13px] leading-normal text-placeholder">No pull requests yet.</div>
      )}
      {error && <div className="mt-2.5 text-[13px] leading-snug text-danger">{error}</div>}
    </div>
  );
}
