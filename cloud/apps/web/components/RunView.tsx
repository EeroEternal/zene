"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { api } from "@/lib/api";
import type {
  Approval,
  PullRequest,
  Repo,
  Run,
  RunEvent,
  RunMessage,
  WorkspaceFile,
} from "@/lib/types";
import { repoLabel } from "./Sidebar";
import { useToast } from "./Toast";

type TimelineItem =
  | { kind: "bubble"; id: number; role: string; text: string }
  | { kind: "approval"; id: number; approval: Approval; decision?: string };

type Tab = "overview" | "files" | "changes" | "pr";

function bubbleRole(role?: string): "user" | "assistant" | "tool" | "event" {
  const r = (role || "assistant").toLowerCase();
  if (r === "user") return "user";
  if (r === "tool") return "tool";
  if (r === "system" || r === "event") return "event";
  return "assistant";
}

function DiffView({ diff }: { diff: string }) {
  const lines = diff ? diff.split("\n") : [];
  return (
    <div className="min-h-[80px] whitespace-pre-wrap break-words rounded-lg border border-line bg-tertiary p-2.5 font-mono text-[11px] leading-[1.45] text-ink">
      {!diff
        ? "No changes."
        : lines.map((line, i) => {
            let cls = "";
            if (line.startsWith("+") && !line.startsWith("+++")) cls = "text-ok";
            else if (line.startsWith("-") && !line.startsWith("---")) cls = "text-danger";
            else if (line.startsWith("@@")) cls = "text-muted";
            return (
              <span key={i} className={cls}>
                {line}
                {"\n"}
              </span>
            );
          })}
    </div>
  );
}

interface RunViewProps {
  runId: string;
  repos: Repo[];
  onMeta: (title: string, status: string) => void;
  onRunsChanged: () => void;
}

export function RunView({ runId, repos, onMeta, onRunsChanged }: RunViewProps) {
  const toast = useToast();
  const [run, setRun] = useState<Run | null>(null);
  const [items, setItems] = useState<TimelineItem[]>([]);
  const [eventLog, setEventLog] = useState("");
  const [followUp, setFollowUp] = useState("");
  const [sending, setSending] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [tab, setTab] = useState<Tab>("overview");

  const [files, setFiles] = useState<WorkspaceFile[]>([]);
  const [filesError, setFilesError] = useState("");
  const [selectedFile, setSelectedFile] = useState<string | null>(null);
  const [fileView, setFileView] = useState<{ path: string; content: string; truncated?: boolean } | null>(null);
  const [diff, setDiff] = useState("");
  const [diffError, setDiffError] = useState("");
  const [prs, setPrs] = useState<PullRequest[]>([]);
  const [prError, setPrError] = useState("");
  const [prBusy, setPrBusy] = useState(false);
  const [pushBusy, setPushBusy] = useState(false);

  const nextId = useRef(1);
  const afterSeq = useRef(0);
  const seenApprovals = useRef(new Set<string>());
  const hasAssistantTail = useRef(false);
  const messagesRef = useRef<HTMLDivElement>(null);
  const eventLogRef = useRef<HTMLDivElement>(null);

  const scrollMessages = useCallback(() => {
    requestAnimationFrame(() => {
      const el = messagesRef.current;
      if (el) el.scrollTop = el.scrollHeight;
    });
  }, []);

  const appendBubble = useCallback(
    (role: string, text: string) => {
      const r = bubbleRole(role);
      setItems((prev) => [...prev, { kind: "bubble", id: nextId.current++, role: r, text }]);
      hasAssistantTail.current = r === "assistant";
      scrollMessages();
    },
    [scrollMessages],
  );

  const appendAssistantChunk = useCallback(
    (text: string) => {
      if (!text) return;
      setItems((prev) => {
        if (hasAssistantTail.current) {
          for (let i = prev.length - 1; i >= 0; i--) {
            const it = prev[i];
            if (it.kind === "bubble" && it.role === "assistant") {
              const copy = [...prev];
              copy[i] = { ...it, text: it.text + text };
              return copy;
            }
            if (it.kind === "bubble" && it.role === "user") break;
          }
        }
        hasAssistantTail.current = true;
        return [...prev, { kind: "bubble", id: nextId.current++, role: "assistant", text }];
      });
      scrollMessages();
    },
    [scrollMessages],
  );

  const handleEvent = useCallback(
    (event: RunEvent, live: boolean) => {
      const line = `#${event.seq} ${event.eventType || event.event_type || "?"}`;
      setEventLog((prev) => (prev ? prev + "\n" : "") + line);
      requestAnimationFrame(() => {
        const el = eventLogRef.current;
        if (el) el.scrollTop = el.scrollHeight;
      });

      const payload = event.payload || {};
      if (payload.event === "run.status" && payload.status) {
        setRun((prev) => {
          if (!prev) return prev;
          const next = { ...prev, status: payload.status! };
          if (payload.headSha) next.headSha = payload.headSha;
          return next;
        });
        onRunsChanged();
      }

      const update = payload.params?.update;
      if (update) {
        if (update.sessionUpdate === "agent_message_chunk" && live) {
          appendAssistantChunk(update.content?.text || "");
        } else if (update.sessionUpdate === "tool_call") {
          hasAssistantTail.current = false;
          setItems((prev) => [
            ...prev,
            {
              kind: "bubble",
              id: nextId.current++,
              role: "tool",
              text: `${update.title || update.toolName || "tool"} · ${update.status || ""}`,
            },
          ]);
          scrollMessages();
        }
      }
    },
    [appendAssistantChunk, onRunsChanged, scrollMessages],
  );

  const refreshApprovals = useCallback(async () => {
    try {
      const list = (await api<Approval[]>(`/api/v1/runs/${runId}/approvals`)) || [];
      for (const ap of list) {
        if ((ap.status || "").toLowerCase() === "pending" && !seenApprovals.current.has(ap.id)) {
          seenApprovals.current.add(ap.id);
          setItems((prev) => [...prev, { kind: "approval", id: nextId.current++, approval: ap }]);
          scrollMessages();
        }
      }
    } catch (err) {
      console.warn(err);
    }
  }, [runId, scrollMessages]);

  const loadFiles = useCallback(async () => {
    try {
      setFiles((await api<WorkspaceFile[]>(`/api/v1/runs/${runId}/files`)) || []);
      setFilesError("");
    } catch (err) {
      setFiles([]);
      setFilesError(err instanceof Error ? err.message : String(err));
    }
  }, [runId]);

  const loadDiff = useCallback(async () => {
    try {
      const data = await api<{ diff?: string }>(`/api/v1/runs/${runId}/diff`);
      setDiff(((data && data.diff) || "").trim());
      setDiffError("");
    } catch (err) {
      setDiffError(err instanceof Error ? err.message : String(err));
    }
  }, [runId]);

  const loadPrs = useCallback(async () => {
    setPrError("");
    try {
      setPrs((await api<PullRequest[]>(`/api/v1/runs/${runId}/pull-requests`)) || []);
    } catch (err) {
      setPrError(err instanceof Error ? err.message : String(err));
    }
  }, [runId]);

  const openFile = useCallback(
    async (path: string) => {
      setSelectedFile(path);
      try {
        const data = await api<{ path: string; content?: string; truncated?: boolean }>(
          `/api/v1/runs/${runId}/file?path=${encodeURIComponent(path)}`,
        );
        setFileView({ path: data.path, content: data.content || "", truncated: data.truncated });
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [runId, toast],
  );

  // Initial load + polling
  useEffect(() => {
    let stopped = false;
    (async () => {
      try {
        const r = await api<Run>(`/api/v1/runs/${runId}`);
        if (stopped) return;
        setRun(r);
        const msgs = (await api<RunMessage[]>(`/api/v1/runs/${runId}/messages`)) || [];
        if (stopped) return;
        setItems(
          msgs.map((m) => ({ kind: "bubble" as const, id: nextId.current++, role: bubbleRole(m.role), text: m.content })),
        );
        hasAssistantTail.current = false;
        const hist = await api<{ events?: RunEvent[]; nextSeq?: number }>(`/api/v1/runs/${runId}/events?afterSeq=0`);
        if (stopped) return;
        for (const e of hist.events || []) handleEvent(e, false);
        afterSeq.current = hist.nextSeq || afterSeq.current;
        await refreshApprovals();
        onRunsChanged();
        scrollMessages();
      } catch (err) {
        if (!stopped) toast(err instanceof Error ? err.message : String(err), "error");
      }
    })();

    const pollTimer = setInterval(async () => {
      try {
        const hist = await api<{ events?: RunEvent[]; nextSeq?: number }>(
          `/api/v1/runs/${runId}/events?afterSeq=${afterSeq.current}`,
        );
        for (const e of hist.events || []) {
          handleEvent(e, true);
          afterSeq.current = e.seq;
        }
        if (hist.nextSeq != null) afterSeq.current = hist.nextSeq;
      } catch (err) {
        console.warn(err);
      }
    }, 1000);
    const approvalTimer = setInterval(refreshApprovals, 2000);

    return () => {
      stopped = true;
      clearInterval(pollTimer);
      clearInterval(approvalTimer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runId]);

  useEffect(() => {
    onMeta(run?.title || "Agent", run?.status || "idle");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run?.title, run?.status]);

  useEffect(() => {
    if (tab === "files") loadFiles();
    if (tab === "changes") loadDiff();
    if (tab === "pr") loadPrs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab]);

  const sendFollowUp = useCallback(async () => {
    const text = followUp.trim();
    if (!text) return;
    setSending(true);
    try {
      appendBubble("user", text);
      setFollowUp("");
      await api(`/api/v1/runs/${runId}/messages`, {
        method: "POST",
        body: JSON.stringify({ text, clientMessageId: crypto.randomUUID() }),
      });
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setSending(false);
    }
  }, [followUp, runId, appendBubble, toast]);

  const cancelRun = useCallback(async () => {
    setCancelling(true);
    try {
      const r = await api<Run>(`/api/v1/runs/${runId}/cancel`, { method: "POST", body: "{}" });
      setRun(r);
      onRunsChanged();
      toast("Run stopped", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setCancelling(false);
    }
  }, [runId, onRunsChanged, toast]);

  const decideApproval = useCallback(
    async (itemId: number, approvalId: string, decision: string) => {
      try {
        await api(`/api/v1/runs/${runId}/approvals/${approvalId}/decide`, {
          method: "POST",
          body: JSON.stringify({ decision }),
        });
        setItems((prev) => prev.map((it) => (it.id === itemId && it.kind === "approval" ? { ...it, decision } : it)));
        toast(`Approval: ${decision}`, "ok");
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [runId, toast],
  );

  const createPr = useCallback(async () => {
    setPrError("");
    setPrBusy(true);
    try {
      await api(`/api/v1/runs/${runId}/pull-requests`, {
        method: "POST",
        body: JSON.stringify({
          title: run?.title || "Zene Cloud PR",
          body: "Created by Zene Cloud Agent",
          draft: true,
        }),
      });
      toast("Pull request created", "ok");
      await loadPrs();
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setPrError(msg);
      toast(msg, "error");
    } finally {
      setPrBusy(false);
    }
  }, [runId, run?.title, loadPrs, toast]);

  const pushBranch = useCallback(async () => {
    setPrError("");
    setPushBusy(true);
    try {
      const result = await api<{ headSha?: string; pushUrl?: string }>(`/api/v1/runs/${runId}/git/push`, {
        method: "POST",
        body: "{}",
      });
      toast(`Pushed · ${result.headSha || result.pushUrl || "ok"}`, "ok");
      if (result.headSha) setRun((prev) => (prev ? { ...prev, headSha: result.headSha } : prev));
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      setPrError(msg);
      toast(msg, "error");
    } finally {
      setPushBusy(false);
    }
  }, [runId, toast]);

  const kv: [string, string][] = useMemo(
    () => [
      ["ID", run?.id || "—"],
      ["Repo", run ? repoLabel(repos, run.repositoryId) : "—"],
      ["Branch", run?.headBranch || "—"],
      ["Base", run?.baseRef || "—"],
      ["Model", run?.model || "—"],
      ["Mode", run?.permissionMode || "—"],
      ["Status", run?.status || "—"],
      ["Head SHA", run?.headSha || "—"],
    ],
    [run, repos],
  );

  const tabs: { id: Tab; label: string }[] = [
    { id: "overview", label: "Overview" },
    { id: "files", label: "Files" },
    { id: "changes", label: "Changes" },
    { id: "pr", label: "PR" },
  ];

  return (
    <div className="h-full">
      <div className="grid h-full grid-cols-1 grid-rows-[minmax(0,1fr)_auto] bg-canvas p-2 min-[981px]:grid-cols-[minmax(0,1fr)_320px] min-[981px]:grid-rows-1 min-[981px]:p-4">
        <div className="grid min-w-0 grid-rows-[minmax(0,1fr)_auto] overflow-hidden rounded-t-xl border border-line bg-canvas shadow-card min-[981px]:rounded-l-xl min-[981px]:rounded-tr-none">
          <div ref={messagesRef} className="flex flex-col gap-3 overflow-auto bg-canvas px-[22px] pb-3 pt-5">
            {items.map((item) => {
              if (item.kind === "approval") {
                const ap = item.approval;
                const summary =
                  typeof ap.payload === "object" ? JSON.stringify(ap.payload, null, 2) : String(ap.payload || "");
                const allowed = ap.allowedDecisions || ["allow-once", "reject-once"];
                const extra = allowed.filter((d) => !["allow-once", "reject-once", "allow", "deny"].includes(d));
                const decided = Boolean(item.decision);
                return (
                  <div
                    key={item.id}
                    className={`self-stretch rounded-lg border border-[#E8D5A8] bg-warn-soft p-3.5 min-[981px]:ml-9 ${decided ? "opacity-[.65]" : ""}`}
                  >
                    <h4 className="mb-1.5 text-[13px] font-semibold text-warn-ink">
                      Permission · {ap.kind || "tool"} · {ap.risk || ""}
                      {item.decision ? ` · ${item.decision}` : ""}
                    </h4>
                    <div className="mb-3 max-h-40 overflow-auto whitespace-pre-wrap break-words rounded-md border border-line bg-canvas p-2.5 font-mono text-xs text-muted">
                      {summary}
                    </div>
                    <div className="flex flex-wrap gap-2">
                      {(allowed.includes("allow-once") || allowed.includes("allow")) && (
                        <button
                          type="button"
                          className="btn btn-primary btn-sm"
                          disabled={decided}
                          onClick={() => decideApproval(item.id, ap.id, "allow-once")}
                        >
                          Allow once
                        </button>
                      )}
                      {(allowed.includes("reject-once") || allowed.includes("deny")) && (
                        <button
                          type="button"
                          className="btn btn-danger btn-sm"
                          disabled={decided}
                          onClick={() => decideApproval(item.id, ap.id, "reject-once")}
                        >
                          Reject once
                        </button>
                      )}
                      {extra.map((d) => (
                        <button
                          key={d}
                          type="button"
                          className="btn btn-sm"
                          disabled={decided}
                          onClick={() => decideApproval(item.id, ap.id, d)}
                        >
                          {d}
                        </button>
                      ))}
                    </div>
                  </div>
                );
              }
              const role = item.role;
              if (role === "user") {
                return (
                  <div
                    key={item.id}
                    className="max-w-[78%] self-end whitespace-pre-wrap break-words rounded-xl rounded-br-[4px] bg-ink px-3.5 py-2.5 text-[13.5px] leading-[1.55] text-white"
                  >
                    <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-[0.04em] text-placeholder">user</div>
                    {item.text}
                  </div>
                );
              }
              if (role === "tool" || role === "event") {
                return (
                  <div
                    key={item.id}
                    className="ml-9 self-stretch whitespace-pre-wrap break-words rounded-lg border border-line bg-tertiary px-3 py-2.5 font-mono text-[11px] leading-[1.55] text-muted"
                  >
                    <div className="mb-1.5 font-sans text-[10px] font-semibold uppercase tracking-[0.04em] text-placeholder">
                      {role}
                    </div>
                    {item.text}
                  </div>
                );
              }
              return (
                <div
                  key={item.id}
                  className="relative max-w-[min(780px,92%)] self-start whitespace-pre-wrap break-words pl-9 text-[13.5px] leading-[1.55] text-ink before:absolute before:left-0 before:top-0 before:grid before:h-[26px] before:w-[26px] before:place-items-center before:rounded-[7px] before:bg-secondary before:text-xs before:font-bold before:text-ink before:content-['Z']"
                >
                  <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-[0.04em] text-placeholder">
                    assistant
                  </div>
                  {item.text}
                </div>
              );
            })}
          </div>
          <div className="grid grid-cols-[1fr_auto] gap-2.5 border-t border-line bg-canvas px-3.5 py-3">
            <textarea
              className="field-input max-h-40 min-h-[52px] resize-none"
              rows={2}
              placeholder="Follow-up instructions…"
              value={followUp}
              onChange={(e) => setFollowUp(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  sendFollowUp();
                }
              }}
            />
            <div className="flex flex-col gap-2">
              <button type="button" className="btn btn-primary" disabled={sending} onClick={sendFollowUp}>
                Send
              </button>
              <button type="button" className="btn btn-danger" disabled={cancelling} onClick={cancelRun}>
                Stop
              </button>
            </div>
          </div>
        </div>

        <aside className="grid min-h-0 max-h-[42vh] grid-rows-[48px_minmax(0,1fr)] overflow-hidden rounded-b-xl border border-t-0 border-line bg-canvas min-[981px]:max-h-none min-[981px]:rounded-r-xl min-[981px]:rounded-bl-none min-[981px]:border-l-0 min-[981px]:border-t">
          <div className="flex h-12 items-stretch gap-1 overflow-x-auto border-b border-line px-3" role="tablist">
            {tabs.map((t) => (
              <button
                key={t.id}
                type="button"
                className={`h-full whitespace-nowrap border-b-2 px-2.5 text-[13px] font-medium ${
                  tab === t.id ? "border-ink text-ink" : "border-transparent text-muted hover:text-ink"
                }`}
                onClick={() => setTab(t.id)}
              >
                {t.label}
              </button>
            ))}
          </div>
          <div className="min-h-0 overflow-auto px-4 pb-5 pt-3.5">
            {tab === "overview" && (
              <div>
                <h3 className="panel-title">Run</h3>
                {kv.map(([k, v]) => (
                  <div key={k} className="mb-2 grid grid-cols-[88px_minmax(0,1fr)] items-start gap-2 text-xs">
                    <span className="text-muted">{k}</span>
                    <code className="break-all font-mono text-[11px] text-ink">{v}</code>
                  </div>
                ))}
                <h3 className="panel-title mt-[18px]">Events</h3>
                <div
                  ref={eventLogRef}
                  className="max-h-[220px] overflow-auto whitespace-pre-wrap rounded-lg border border-line bg-tertiary p-2.5 font-mono text-[11px] leading-[1.45] text-muted"
                >
                  {eventLog}
                </div>
              </div>
            )}
            {tab === "files" && (
              <div>
                <div className="mb-2 flex items-center justify-between">
                  <h3 className="panel-title !mb-0">Workspace files</h3>
                  <button type="button" className="btn btn-sm" onClick={loadFiles}>
                    Refresh
                  </button>
                </div>
                <div className="flex flex-col gap-0.5">
                  {files.map((f) => (
                    <button
                      key={f.path}
                      type="button"
                      className={[
                        "overflow-hidden text-ellipsis whitespace-nowrap rounded-md px-2 py-1.5 text-left font-mono text-[11px]",
                        f.kind === "dir"
                          ? "cursor-default text-placeholder"
                          : "text-muted hover:bg-secondary hover:text-ink",
                        selectedFile === f.path && f.kind === "file" ? "bg-secondary text-ink" : "",
                      ].join(" ")}
                      onClick={f.kind === "file" ? () => openFile(f.path) : undefined}
                    >
                      {(f.kind === "dir" ? "▸ " : "") + f.path + (f.kind === "file" && f.size != null ? `  (${f.size})` : "")}
                    </button>
                  ))}
                </div>
                {fileView && (
                  <div className="mt-3 overflow-hidden rounded-lg border border-line bg-tertiary">
                    <div className="border-b border-line px-2.5 py-2 font-mono text-[11px] text-muted">
                      {fileView.path}
                      {fileView.truncated ? " (truncated)" : ""}
                    </div>
                    <pre className="m-0 max-h-[420px] overflow-auto whitespace-pre-wrap break-words p-2.5 font-mono text-[11px] leading-[1.45] text-ink">
                      {fileView.content}
                    </pre>
                  </div>
                )}
                {!files.length && (
                  <div className="py-3 text-[13px] leading-normal text-placeholder">
                    {filesError || "No workspace files yet."}
                  </div>
                )}
              </div>
            )}
            {tab === "changes" && (
              <div>
                <div className="mb-2 flex items-center justify-between">
                  <h3 className="panel-title !mb-0">Diff</h3>
                  <button type="button" className="btn btn-sm" onClick={loadDiff}>
                    Refresh
                  </button>
                </div>
                {diffError ? (
                  <div className="min-h-[80px] rounded-lg border border-line bg-tertiary p-2.5 font-mono text-[11px] text-ink">
                    {diffError}
                  </div>
                ) : (
                  <DiffView diff={diff} />
                )}
              </div>
            )}
            {tab === "pr" && (
              <div>
                <h3 className="panel-title">Pull requests</h3>
                <div className="my-3 flex flex-wrap gap-2">
                  <button type="button" className="btn btn-primary btn-sm" disabled={prBusy} onClick={createPr}>
                    Create PR
                  </button>
                  <button type="button" className="btn btn-sm" disabled={pushBusy} onClick={pushBranch}>
                    Push
                  </button>
                  <button type="button" className="btn btn-sm" onClick={loadPrs}>
                    Refresh
                  </button>
                </div>
                <div>
                  {prs.map((pr, i) => (
                    <div key={i} className="mb-2.5 rounded-lg border border-line bg-canvas p-3">
                      <div className="mb-1 text-[13px] font-semibold">
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
                          pr.title
                        )}
                      </div>
                      <div className="font-mono text-[11px] text-muted">
                        #{pr.providerNumber ?? "—"} · {pr.state}
                        {pr.draft ? " · draft" : ""}
                      </div>
                    </div>
                  ))}
                </div>
                {!prs.length && !prError && (
                  <div className="py-3 text-[13px] leading-normal text-placeholder">No pull requests yet.</div>
                )}
                <div className="mt-2.5 min-h-[18px] text-[13px] leading-snug text-danger">{prError}</div>
              </div>
            )}
          </div>
        </aside>
      </div>
    </div>
  );
}
