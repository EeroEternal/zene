"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { api } from "@/lib/api";
import {
  IconArrowUp,
  IconBranch,
  IconCheck,
  IconChevronDown,
  IconLoader,
  IconPanelRight,
  IconPlus,
  IconSearch,
  IconStop,
} from "@/lib/icons";
import {
  DEFAULT_MODEL_ID,
  loadSelectedModel,
  modelLabel,
  modelsForPicker,
  saveSelectedModel,
} from "@/lib/models";
import type { Approval, LlmSettingsView, Repo, Run, RunEvent, RunMessage } from "@/lib/types";
import { CodePanel, useCodePanelWidth } from "./CodePanel";
import { repoLabel } from "./Sidebar";
import { StatusPill } from "./StatusPill";
import { useToast } from "./Toast";

const ACTIVE_STATUSES = new Set([
  "running",
  "starting",
  "cloning",
  "provisioning",
  "queued",
  "waiting_for_approval",
  "waiting_for_user",
]);

const SETUP_STATUSES = new Set(["queued", "provisioning", "starting", "cloning"]);

function setupStatusCopy(status: string, repo?: string): { title: string; detail: string } {
  const repoLabel = repo && repo !== "—" ? repo : "repository";
  switch (status) {
    case "cloning":
      return {
        title: `Cloning ${repoLabel}`,
        detail:
          "Downloading the repository into a local workspace. Large repos can take several minutes — the agent starts after clone finishes.",
      };
    case "queued":
      return {
        title: "Waiting for a worker",
        detail: "Your task is queued. A worker will pick it up shortly.",
      };
    case "provisioning":
    case "starting":
      return {
        title: "Starting agent",
        detail: "Preparing the workspace and launching the coding agent.",
      };
    default:
      return {
        title: status,
        detail: "Setting up your agent session…",
      };
  }
}

type TimelineItem =
  | { kind: "bubble"; id: number; role: string; text: string }
  | { kind: "approval"; id: number; approval: Approval; decision?: string };

function bubbleRole(role?: string): "user" | "assistant" | "tool" | "event" {
  const r = (role || "assistant").toLowerCase();
  if (r === "user") return "user";
  if (r === "tool") return "tool";
  if (r === "system" || r === "event") return "event";
  return "assistant";
}

interface RunViewProps {
  runId: string;
  repos: Repo[];
  codePanelOpen?: boolean;
  onToggleCodePanel?: () => void;
  onOpenMenu?: () => void;
  onMeta: (title: string, status: string) => void;
  onRunsChanged: () => void;
}

export function RunView({
  runId,
  repos,
  codePanelOpen = true,
  onToggleCodePanel,
  onOpenMenu,
  onMeta,
  onRunsChanged,
}: RunViewProps) {
  const toast = useToast();
  const { width: codeWidth, setWidth: setCodeWidth } = useCodePanelWidth();
  const [run, setRun] = useState<Run | null>(null);
  const [items, setItems] = useState<TimelineItem[]>([]);
  const [followUp, setFollowUp] = useState("");
  const [sending, setSending] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [selectedModel, setSelectedModel] = useState(DEFAULT_MODEL_ID);
  const [modelMenuOpen, setModelMenuOpen] = useState(false);
  const [modelQuery, setModelQuery] = useState("");
  const [llmSettings, setLlmSettings] = useState<LlmSettingsView | null>(null);

  const nextId = useRef(1);
  const afterSeq = useRef(0);
  const seenApprovals = useRef(new Set<string>());
  const hasAssistantTail = useRef(false);
  const messagesRef = useRef<HTMLDivElement>(null);
  const promptRef = useRef<HTMLTextAreaElement>(null);
  const composerRef = useRef<HTMLDivElement>(null);

  const scrollMessages = useCallback(() => {
    requestAnimationFrame(() => {
      const el = messagesRef.current;
      if (el) el.scrollTop = el.scrollHeight;
    });
  }, []);

  const appendBubble = useCallback(
    (role: string, text: string) => {
      const r = bubbleRole(role);
      const id = nextId.current++;
      setItems((prev) => [...prev, { kind: "bubble", id, role: r, text }]);
      hasAssistantTail.current = r === "assistant";
      scrollMessages();
      return id;
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
    (event: RunEvent) => {
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
        if (update.sessionUpdate === "agent_message_chunk") {
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

  useEffect(() => {
    let stopped = false;
    (async () => {
      try {
        const r = await api<Run>(`/api/v1/runs/${runId}`);
        if (stopped) return;
        setRun(r);
        const msgs = (await api<RunMessage[]>(`/api/v1/runs/${runId}/messages`)) || [];
        if (stopped) return;
        const hist = await api<{ events?: RunEvent[]; nextSeq?: number }>(
          `/api/v1/runs/${runId}/events?afterSeq=0`,
        );
        if (stopped) return;
        setItems([]);
        hasAssistantTail.current = false;
        const history = [
          ...msgs.map((message) => ({
            kind: "message" as const,
            createdAt: message.createdAt,
            message,
            seq: -1,
          })),
          ...(hist.events || []).map((event) => ({
            kind: "event" as const,
            createdAt: event.createdAt,
            event,
            seq: event.seq,
          })),
        ].sort(
          (a, b) =>
            a.createdAt.localeCompare(b.createdAt) ||
            (a.kind === b.kind ? a.seq - b.seq : a.kind === "message" ? -1 : 1),
        );
        for (const entry of history) {
          if (entry.kind === "message") {
            appendBubble(entry.message.role, entry.message.content);
          } else {
            handleEvent(entry.event);
          }
        }
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
          handleEvent(e);
          afterSeq.current = e.seq;
        }
        if (hist.nextSeq != null) afterSeq.current = hist.nextSeq;
      } catch (err) {
        console.warn(err);
      }
    }, 1000);
    const approvalTimer = setInterval(refreshApprovals, 2000);
    const statusTimer = setInterval(async () => {
      try {
        const r = await api<Run>(`/api/v1/runs/${runId}`);
        if (stopped) return;
        setRun((prev) => {
          if (!prev) return r;
          if (prev.status === r.status && prev.headSha === r.headSha) return prev;
          return { ...prev, ...r };
        });
        if (SETUP_STATUSES.has((r.status || "").toLowerCase())) onRunsChanged();
      } catch (err) {
        console.warn(err);
      }
    }, 2000);

    return () => {
      stopped = true;
      clearInterval(pollTimer);
      clearInterval(approvalTimer);
      clearInterval(statusTimer);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runId]);

  useEffect(() => {
    onMeta(run?.title || "Agent", run?.status || "idle");
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run?.title, run?.status]);

  useEffect(() => {
    setSelectedModel(loadSelectedModel());
    api<LlmSettingsView>("/api/v1/settings/llm")
      .then(setLlmSettings)
      .catch(() => setLlmSettings(null));
  }, []);

  useEffect(() => {
    if (!modelMenuOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (composerRef.current && !composerRef.current.contains(e.target as Node)) {
        setModelMenuOpen(false);
        setModelQuery("");
      }
    };
    document.addEventListener("mousedown", onDoc);
    return () => document.removeEventListener("mousedown", onDoc);
  }, [modelMenuOpen]);

  const pickerModels = useMemo(() => modelsForPicker(llmSettings), [llmSettings]);
  const filteredModels = useMemo(() => {
    const q = modelQuery.trim().toLowerCase();
    if (!q) return pickerModels;
    return pickerModels.filter((m) => m.toLowerCase().includes(q));
  }, [pickerModels, modelQuery]);

  const repoName = run ? repoLabel(repos, run.repositoryId) : "";
  const statusKey = (run?.status || "").toLowerCase();
  const isActive = ACTIVE_STATUSES.has(statusKey);
  const isSetup = SETUP_STATUSES.has(statusKey);
  const setupCopy = isSetup ? setupStatusCopy(statusKey, repoName) : null;
  const cannotSend =
    !run ||
    ["created", "stopping", "failed", "timed_out", "cancelled"].includes(statusKey);
  const canSend = Boolean(followUp.trim()) && !sending && !cannotSend;

  const autosize = useCallback(() => {
    const el = promptRef.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(128, Math.max(32, el.scrollHeight))}px`;
  }, []);

  const sendFollowUp = useCallback(async () => {
    const text = followUp.trim();
    if (!text || sending || cannotSend) return;
    const optimisticId = appendBubble("user", text);
    setSending(true);
    setFollowUp("");
    try {
      await api(`/api/v1/runs/${runId}/messages`, {
        method: "POST",
        body: JSON.stringify({ text, clientMessageId: crypto.randomUUID() }),
      });
    } catch (err) {
      setItems((prev) => prev.filter((item) => item.id !== optimisticId));
      setFollowUp(text);
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setSending(false);
    }
  }, [followUp, sending, cannotSend, runId, appendBubble, toast]);

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
        setItems((prev) =>
          prev.map((it) => (it.id === itemId && it.kind === "approval" ? { ...it, decision } : it)),
        );
        toast(`Approval: ${decision}`, "ok");
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      }
    },
    [runId, toast],
  );

  return (
    <div
      className={[
        "relative grid h-full min-h-0 grid-cols-1",
        codePanelOpen
          ? "grid-rows-[minmax(0,1fr)_minmax(0,38vh)] min-[981px]:grid-rows-1 min-[981px]:grid-cols-[minmax(0,1fr)_var(--code-panel-w)]"
          : "grid-rows-1",
      ].join(" ")}
      style={{ ["--code-panel-w" as string]: `${codeWidth}px` } as CSSProperties}
    >
        {/* 中：title + 对话 + Prompt（白底） */}
        <div className="grid min-h-0 min-w-0 grid-rows-[36px_minmax(0,1fr)_auto] overflow-hidden bg-canvas">
          <header className="flex h-9 items-center gap-2 border-b border-line px-3">
            <button
              type="button"
              className="hidden h-7 w-7 shrink-0 items-center justify-center rounded-md text-muted hover:bg-secondary max-[980px]:inline-flex"
              aria-label="Open menu"
              onClick={onOpenMenu}
            >
              ☰
            </button>
            <h1 className="min-w-0 truncate text-[13px] font-semibold text-ink">
              {run?.title || "Agent"}
            </h1>
            {repoName && repoName !== "—" && (
              <span className="hidden min-w-0 truncate text-[12px] text-muted min-[720px]:inline">
                {repoName}
              </span>
            )}
            {run?.status && <StatusPill status={run.status} />}
            <div className="ml-auto flex shrink-0 items-center gap-1">
              {!codePanelOpen && (
                <button
                  type="button"
                  className="hidden h-7 w-7 items-center justify-center rounded-md text-muted hover:bg-secondary hover:text-ink min-[981px]:inline-flex"
                  title="Show panel"
                  aria-label="Show panel"
                  onClick={onToggleCodePanel}
                >
                  <IconPanelRight className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
          </header>
          <div
            ref={messagesRef}
            className="flex flex-col gap-2 overflow-auto px-3 pb-1.5 pt-2"
          >
            <div className="mx-auto flex w-full max-w-[620px] flex-col gap-2">
              {setupCopy && (
                <div
                  className="flex items-start gap-3 self-stretch rounded-xl border border-line bg-tertiary px-3.5 py-3"
                  role="status"
                  aria-live="polite"
                >
                  <IconLoader className="mt-0.5 h-4 w-4 shrink-0 animate-spin text-ink" />
                  <div className="min-w-0">
                    <div className="text-[13.5px] font-semibold text-ink">{setupCopy.title}</div>
                    <p className="m-0 mt-1 text-[12.5px] leading-[1.45] text-muted">{setupCopy.detail}</p>
                  </div>
                </div>
              )}
              {items.map((item) => {
                if (item.kind === "approval") {
                  const ap = item.approval;
                  const summary =
                    typeof ap.payload === "object"
                      ? JSON.stringify(ap.payload, null, 2)
                      : String(ap.payload || "");
                  const allowed = ap.allowedDecisions || ["allow-once", "reject-once"];
                  const extra = allowed.filter(
                    (d) => !["allow-once", "reject-once", "allow", "deny"].includes(d),
                  );
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
                      <div className="mb-1.5 text-[10px] font-semibold uppercase tracking-[0.04em] text-placeholder">
                        user
                      </div>
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
          </div>
          <div ref={composerRef} className="border-t border-line bg-canvas px-3 pb-2.5 pt-2">
            <div className="mx-auto w-full max-w-[620px]">
              <div className="rounded-[10px] border border-line-strong bg-tertiary px-2 pb-1.5 pt-1.5 focus-within:border-ink/30 focus-within:bg-canvas">
                <textarea
                  ref={promptRef}
                  className="block max-h-32 min-h-[32px] w-full resize-none border-0 bg-transparent px-0.5 pb-1 pt-0 text-[13px] leading-normal text-ink outline-none"
                  rows={1}
                  placeholder="Send follow-up…"
                  aria-label="Follow-up"
                  value={followUp}
                  onChange={(e) => {
                    setFollowUp(e.target.value);
                    autosize();
                  }}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
                      e.preventDefault();
                      if (canSend) sendFollowUp();
                    }
                  }}
                />
                <div className="flex items-center justify-between gap-2">
                  <div className="flex min-w-0 items-center gap-1">
                    <button
                      type="button"
                      className="inline-flex h-6 w-6 items-center justify-center rounded-full bg-secondary text-muted hover:bg-active hover:text-ink"
                      title="Add"
                      aria-label="Add"
                      onClick={() => toast("Attachments coming soon", "ok")}
                    >
                      <IconPlus className="h-3.5 w-3.5" />
                    </button>
                    <div className="relative">
                      <button
                        type="button"
                        className="inline-flex h-6 max-w-[200px] items-center gap-1 rounded-md px-1.5 text-[12px] font-medium text-muted hover:bg-secondary hover:text-ink"
                        title="Model"
                        aria-label="Model"
                        aria-haspopup="menu"
                        aria-expanded={modelMenuOpen}
                        onClick={() => {
                          setModelMenuOpen((o) => !o);
                          setModelQuery("");
                        }}
                      >
                        <span className="min-w-0 overflow-hidden text-ellipsis whitespace-nowrap">
                          {modelLabel(selectedModel)}
                        </span>
                        <IconChevronDown className="h-3 w-3 shrink-0" />
                      </button>
                      {modelMenuOpen && (
                        <div
                          className="absolute bottom-[calc(100%+8px)] left-0 z-[45] w-[min(280px,calc(100vw-48px))] overflow-hidden rounded-xl border border-line bg-canvas shadow-menu"
                          role="menu"
                          aria-label="Models"
                        >
                          <div className="flex items-center gap-2 border-b border-line px-3 py-2">
                            <IconSearch className="h-3.5 w-3.5 shrink-0 text-placeholder" />
                            <input
                              className="min-w-0 flex-1 border-0 bg-transparent text-[13px] outline-none"
                              type="search"
                              placeholder="Search models"
                              autoComplete="off"
                              autoFocus
                              value={modelQuery}
                              onChange={(e) => setModelQuery(e.target.value)}
                            />
                          </div>
                          <div className="max-h-[280px] overflow-auto p-1.5">
                            {!filteredModels.length ? (
                              <p className="m-0 px-2 py-1.5 text-xs text-muted">No models — configure in Settings</p>
                            ) : (
                              filteredModels.map((m) => (
                                <button
                                  key={m}
                                  type="button"
                                  className="picker-item"
                                  onClick={() => {
                                    setSelectedModel(m);
                                    saveSelectedModel(m);
                                    setModelMenuOpen(false);
                                    setModelQuery("");
                                  }}
                                >
                                  <span className="min-w-0 flex-1 overflow-hidden text-ellipsis whitespace-nowrap font-mono text-[12.5px]">
                                    {m}
                                  </span>
                                  {m === selectedModel && (
                                    <IconCheck className="h-3.5 w-3.5 shrink-0 text-ink" />
                                  )}
                                </button>
                              ))
                            )}
                          </div>
                        </div>
                      )}
                    </div>
                    <span className="ml-0.5 hidden items-center gap-1 text-[11px] text-placeholder min-[640px]:inline-flex">
                      <IconBranch className="h-3 w-3" />
                      <span className="max-w-[140px] truncate font-mono">{run?.headBranch || "—"}</span>
                    </span>
                  </div>
                  <div className="flex shrink-0 items-center gap-1">
                    {isActive && (
                      <button
                        type="button"
                        className="inline-flex h-6 w-6 items-center justify-center rounded-sm border border-line-strong bg-canvas text-muted hover:bg-secondary hover:text-ink disabled:opacity-35"
                        title="Stop"
                        aria-label="Stop"
                        disabled={cancelling}
                        onClick={cancelRun}
                      >
                        <IconStop className="h-2.5 w-2.5 fill-current" />
                      </button>
                    )}
                    <button
                      type="button"
                      className="inline-flex h-6 w-6 items-center justify-center rounded-sm bg-primary text-white hover:bg-primary-hover disabled:opacity-35 disabled:hover:bg-primary"
                      title="Send"
                      aria-label="Send"
                      disabled={!canSend}
                      onClick={sendFollowUp}
                    >
                      {sending ? (
                        <IconLoader className="h-3.5 w-3.5 animate-spin" />
                      ) : (
                        <IconArrowUp className="h-3.5 w-3.5" />
                      )}
                    </button>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </div>

        {/* 右：代码 / 文件独立分栏 */}
        {codePanelOpen && (
          <CodePanel
            runId={runId}
            defaultPrTitle={run?.title}
            defaultBaseRef={run?.baseRef}
            headBranch={run?.headBranch}
            width={codeWidth}
            onWidthChange={setCodeWidth}
            onCollapse={onToggleCodePanel}
          />
        )}
    </div>
  );
}
