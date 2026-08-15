"use client";

import { useCallback, useEffect, useState } from "react";
import { createRunPullRequest, fetchRunPullRequests, publishRunToGithub } from "@/lib/gitPublish";
import { buildDefaultPrBody } from "@/lib/prBody";
import type { GitCompare, PullRequest } from "@/lib/types";
import { IconExternal, IconRefresh } from "@/lib/icons";
import { useToast } from "./Toast";

export function PrPanel({
  runId,
  defaultTitle,
  defaultBaseRef,
  headBranch,
  compare,
  dialog,
  onSuccess,
}: {
  runId: string;
  defaultTitle?: string;
  defaultBaseRef?: string;
  headBranch?: string;
  compare?: GitCompare | null;
  /** Submit-only layout for the create-PR dialog. */
  dialog?: boolean;
  onSuccess?: () => void;
}) {
  const toast = useToast();
  const [prs, setPrs] = useState<PullRequest[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const [title, setTitle] = useState(defaultTitle || "");
  const [body, setBody] = useState(() => buildDefaultPrBody(compare));
  const [draft, setDraft] = useState(true);
  const [baseRef, setBaseRef] = useState(defaultBaseRef || "");

  useEffect(() => {
    if (defaultTitle) setTitle((t) => t || defaultTitle);
  }, [defaultTitle]);

  useEffect(() => {
    if (defaultBaseRef) setBaseRef((b) => b || defaultBaseRef);
  }, [defaultBaseRef]);

  useEffect(() => {
    const generated = buildDefaultPrBody(compare);
    if (generated) setBody((b) => b || generated);
  }, [compare]);

  const loadPrs = useCallback(async () => {
    setError("");
    try {
      setPrs(await fetchRunPullRequests(runId));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  }, [runId]);

  useEffect(() => {
    if (!dialog) loadPrs();
  }, [dialog, loadPrs]);

  const publish = useCallback(async () => {
    const t = title.trim();
    if (!t) {
      toast("Title is required", "error");
      return;
    }
    setError("");
    setBusy(true);
    try {
      const result = await publishRunToGithub(runId, {
        title: t,
        baseRef: baseRef.trim() || defaultBaseRef,
        headBranch,
        body: body.trim() || undefined,
        compare,
        draft,
      });
      toast(
        result.pullRequest?.providerNumber != null
          ? `Pushed · PR #${result.pullRequest.providerNumber}`
          : `Pushed · ${result.push.headSha || result.push.pushUrl || "ok"}`,
        "ok",
      );
      if (!dialog) await loadPrs();
      onSuccess?.();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      toast(msg, "error");
    } finally {
      setBusy(false);
    }
  }, [
    runId,
    title,
    defaultBaseRef,
    baseRef,
    headBranch,
    body,
    compare,
    draft,
    dialog,
    loadPrs,
    onSuccess,
    toast,
  ]);

  const createPr = useCallback(async () => {
    const t = title.trim();
    if (!t) {
      toast("Title is required", "error");
      return;
    }
    setError("");
    setBusy(true);
    try {
      await createRunPullRequest(runId, {
        title: t,
        body: body.trim() || buildDefaultPrBody(compare) || undefined,
        draft,
        baseRef: baseRef.trim() || undefined,
        headBranch: headBranch || undefined,
      });
      toast("Pull request created", "ok");
      await loadPrs();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
      toast(msg, "error");
    } finally {
      setBusy(false);
    }
  }, [runId, title, body, compare, draft, baseRef, headBranch, loadPrs, toast]);

  return (
    <div className={`flex h-full min-h-0 flex-col overflow-auto ${dialog ? "px-4 pb-4 pt-3" : "px-4 pb-5 pt-3.5"}`}>
      {!dialog && <h3 className="panel-title">Create pull request</h3>}
      {headBranch && (
        <div className={`font-mono text-[11px] text-muted ${dialog ? "mb-2" : "mb-3"}`}>
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
        className="field-input mb-3 min-h-[120px] resize-y"
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
      <label className={`flex items-center gap-2 text-[13px] text-ink ${dialog ? "mb-4" : "mb-4"}`}>
        <input type="checkbox" checked={draft} onChange={(e) => setDraft(e.target.checked)} />
        Create as draft
      </label>
      <div className={`flex flex-wrap gap-2 ${dialog ? "" : "mb-4"}`}>
        <button type="button" className="btn btn-primary btn-sm" disabled={busy} onClick={publish}>
          {busy ? "Creating…" : dialog ? "Create pull request" : "Push & create PR"}
        </button>
        {!dialog && (
          <>
            <button type="button" className="btn btn-sm" disabled={busy} onClick={createPr}>
              Create PR only
            </button>
            <button type="button" className="btn btn-sm" onClick={loadPrs} title="Refresh" aria-label="Refresh PRs">
              <IconRefresh className="h-3.5 w-3.5" />
            </button>
          </>
        )}
      </div>

      {!dialog && (
        <>
          <h3 className="panel-title">Pull requests</h3>
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
        </>
      )}
      {error && <div className="mt-2.5 text-[13px] leading-snug text-danger">{error}</div>}
    </div>
  );
}
