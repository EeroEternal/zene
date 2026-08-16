"use client";

import { useCallback, useEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { api } from "@/lib/api";
import { IconPlus } from "@/lib/icons";
import {
  DEFAULT_MODEL_ID,
  loadSelectedModel,
  modelsForPicker,
  saveSelectedModel,
} from "@/lib/models";
import type {
  Approval,
  ApprovalDecision,
  GitCompare,
  LlmSettingsView,
  MessageRole,
  PullRequest,
  Repo,
  Run,
  RunEvent,
  RunMessage,
  RunStatus,
} from "@/lib/types";
import { platformEventFromPayload } from "@/lib/platformEvent";
import { timelineProductFromEvent, timelineToolOutput, type TimelineProduct } from "@/lib/runtimeEvent";
import {
  composerChrome,
  isSetupStatus,
  isTerminalStatus,
  sessionPhase,
  setupStatusCopy,
} from "@/lib/sessionPhase";
import {
  bubbleRole,
  buildTimelineFromEvents,
  finalizeTimelineOnStop,
  formatJsonish,
  sealOpenMeta,
  timelineHasLiveMeta,
  type TimelineItem,
} from "@/lib/timeline";
import { buildConversationTurns, buildForkPrompt } from "@/lib/turnActions";
import {
  fetchGitCompare,
  fetchRunPullRequests,
  hasUnpublishedChanges,
  isActivePullRequest,
  type PublishResult,
} from "@/lib/gitPublish";
import { readSessionUi, writeSessionUi } from "@/lib/sessionUi";
import { CodePanel, useCodePanelWidth } from "../CodePanel";
import { PullRequestCard } from "../PullRequestCard";
import { PushPromptCard } from "../PushPromptCard";
import { repoLabel } from "../Sidebar";
import { useToast } from "../Toast";
import { ChatTimeline } from "./ChatTimeline";
import { Composer, type ComposerHandle } from "./composer/Composer";
import type { QueuedPrompt } from "./composer/PromptQueue";
import { SessionHeader } from "./SessionHeader";

interface SessionWorkbenchProps {
  runId: string;
  repos: Repo[];
  codePanelOpen?: boolean;
  onToggleCodePanel?: () => void;
  sidebarCollapsed?: boolean;
  onOpenMenu?: () => void;
  onMeta: (title: string, status?: RunStatus) => void;
  onRename?: (title: string) => Promise<void> | void;
  onRunsChanged: () => void;
  onRunStarted?: (runId: string) => void;
}

export function SessionWorkbench({
  runId,
  repos,
  codePanelOpen = false,
  onToggleCodePanel,
  sidebarCollapsed = false,
  onOpenMenu,
  onMeta,
  onRename,
  onRunsChanged,
  onRunStarted,
}: SessionWorkbenchProps) {
  const toast = useToast();
  const composerRef = useRef<ComposerHandle>(null);
  const { width: codeWidth, setWidth: setCodeWidth } = useCodePanelWidth();
  const [run, setRun] = useState<Run | null>(null);
  const [items, setItems] = useState<TimelineItem[]>([]);
  const [historyReady, setHistoryReady] = useState(false);
  const [followUp, setFollowUp] = useState("");
  const [promptQueue, setPromptQueue] = useState<QueuedPrompt[]>([]);
  const [sending, setSending] = useState(false);
  const [pendingSince, setPendingSince] = useState<number | null>(null);
  const [cancelling, setCancelling] = useState(false);
  const [selectedModel, setSelectedModel] = useState(DEFAULT_MODEL_ID);
  const [llmSettings, setLlmSettings] = useState<LlmSettingsView | null>(null);
  const [editingTitle, setEditingTitle] = useState(false);
  const [titleDraft, setTitleDraft] = useState("");
  const [runMessages, setRunMessages] = useState<RunMessage[]>([]);
  const [forkingTurn, setForkingTurn] = useState<number | null>(null);
  const [retrying, setRetrying] = useState(false);
  const [gitCompare, setGitCompare] = useState<GitCompare | null>(null);
  const [runPullRequests, setRunPullRequests] = useState<PullRequest[]>([]);
  const [pushPublished, setPushPublished] = useState(false);
  const [pushDismissedHead, setPushDismissedHead] = useState<string | null>(() =>
    readSessionUi(runId).pushPromptDismissedHead ?? null,
  );

  const nextId = useRef(1);
  const afterSeq = useRef(0);
  const seenApprovals = useRef(new Set<string>());
  const hasAssistantTail = useRef(false);
  const lastKnownTitle = useRef<string | null>(null);
  const queuedTextsRef = useRef(new Set<string>());

  const appendBubble = useCallback((role: MessageRole, text: string) => {
    const r = bubbleRole(role);
    setItems((prev) => [...prev, { kind: "bubble", id: nextId.current++, role: r, text }]);
    hasAssistantTail.current = r === "assistant";
  }, []);

  const appendAssistantChunk = useCallback((text: string) => {
    if (!text) return;
    setItems((prev) => {
      let base = prev;
      if (!hasAssistantTail.current) {
        base = sealOpenMeta(prev);
      }
      if (hasAssistantTail.current) {
        for (let i = base.length - 1; i >= 0; i--) {
          const it = base[i];
          if (it.kind === "bubble" && it.role === "assistant") {
            const copy = [...base];
            copy[i] = { ...it, text: it.text + text };
            return copy;
          }
          if (it.kind === "bubble" && it.role === "user") break;
        }
      }
      hasAssistantTail.current = true;
      return [...base, { kind: "bubble", id: nextId.current++, role: "assistant", text }];
    });
  }, []);

  const appendThoughtChunk = useCallback((text: string) => {
    if (!text) return;
    hasAssistantTail.current = false;
    setItems((prev) => {
      for (let i = prev.length - 1; i >= 0; i--) {
        const it = prev[i];
        if (it.kind === "thought" && !it.sealed) {
          const copy = [...prev];
          copy[i] = { ...it, text: it.text + text };
          return copy;
        }
        if (it.kind === "bubble" || it.kind === "tool" || it.kind === "approval") break;
      }
      return [
        ...sealOpenMeta(prev),
        {
          kind: "thought",
          id: nextId.current++,
          text,
          expanded: false,
          sealed: false,
          startedAt: Date.now(),
        },
      ];
    });
  }, []);

  const appendUserBubble = useCallback((text: string) => {
    if (!text) return;
    hasAssistantTail.current = false;
    setItems((prev) => {
      for (let i = prev.length - 1; i >= 0; i--) {
        const it = prev[i];
        if (it.kind === "bubble" && it.role === "user" && it.text === text) return prev;
        if (it.kind === "bubble" && it.role === "assistant") break;
      }
      return [...prev, { kind: "bubble", id: nextId.current++, role: "user", text }];
    });
  }, []);

  const upsertToolCall = useCallback((product: TimelineProduct) => {
    const toolCallId = product.toolCallId || `tool-${nextId.current}`;
    const title = product.title || product.toolName || "tool";
    const status = product.status || "pending";
    const input = formatJsonish(product.rawInput);
    hasAssistantTail.current = false;
    setItems((prev) => {
      const idx = prev.findIndex((it) => it.kind === "tool" && it.toolCallId === toolCallId);
      if (idx >= 0) {
        const copy = [...prev];
        const cur = copy[idx] as Extract<TimelineItem, { kind: "tool" }>;
        copy[idx] = {
          ...cur,
          title: title || cur.title,
          toolKind: product.toolKind || cur.toolKind,
          status,
          input: input || cur.input,
        };
        return copy;
      }
      return [
        ...sealOpenMeta(prev),
        {
          kind: "tool",
          id: nextId.current++,
          toolCallId,
          title,
          toolKind: product.toolKind,
          status,
          input: input || undefined,
          expanded: false,
        },
      ];
    });
  }, []);

  const applyToolUpdate = useCallback((product: TimelineProduct) => {
    const toolCallId = product.toolCallId;
    if (!toolCallId) return;
    const status = product.status || "completed";
    const output = timelineToolOutput(product);
    hasAssistantTail.current = false;
    setItems((prev) => {
      const idx = prev.findIndex((it) => it.kind === "tool" && it.toolCallId === toolCallId);
      if (idx < 0) {
        return [
          ...sealOpenMeta(prev),
          {
            kind: "tool",
            id: nextId.current++,
            toolCallId,
            title: product.title || product.toolName || "tool",
            toolKind: product.toolKind,
            status,
            output: output || undefined,
            expanded: false,
          },
        ];
      }
      const copy = [...prev];
      const cur = copy[idx] as Extract<TimelineItem, { kind: "tool" }>;
      copy[idx] = {
        ...cur,
        title: product.title || cur.title,
        toolKind: product.toolKind || cur.toolKind,
        status,
        output: output || cur.output,
      };
      return copy;
    });
  }, []);

  const toggleItem = useCallback((id: number) => {
    setItems((prev) =>
      prev.map((it) => {
        if (it.id !== id) return it;
        if (it.kind === "thought" || it.kind === "tool") {
          return { ...it, expanded: !it.expanded };
        }
        return it;
      }),
    );
  }, []);

  const sealOnStop = useCallback(() => {
    hasAssistantTail.current = false;
    setItems((prev) => finalizeTimelineOnStop(prev));
  }, []);

  const handleEvent = useCallback(
    (event: RunEvent) => {
      const platform = platformEventFromPayload(event.payload);
      if (platform?.event === "run.status" && platform.status) {
        const nextStatus = platform.status!.toLowerCase();
        setRun((prev) => {
          if (!prev) return prev;
          const next = { ...prev, status: platform.status! };
          if (platform.headSha) next.headSha = platform.headSha;
          return next;
        });
        if (isTerminalStatus(nextStatus)) sealOnStop();
        if (nextStatus === "failed" || nextStatus === "timed_out" || nextStatus === "cancelled") {
          setPendingSince(null);
        }
        onRunsChanged();
      }
      if (platform?.event === "run.title" && platform.title) {
        lastKnownTitle.current = platform.title;
        setRun((prev) => (prev ? { ...prev, title: platform.title! } : prev));
        onRunsChanged();
      }

      if (platform?.event === "run.created" && typeof platform.prompt === "string") {
        const prompt = platform.prompt || "";
        if (prompt) {
          hasAssistantTail.current = false;
          setItems((prev) => {
            if (prev.some((it) => it.kind === "bubble" && it.role === "user" && it.text === prompt)) {
              return prev;
            }
            return [...prev, { kind: "bubble", id: nextId.current++, role: "user", text: prompt }];
          });
        }
      }

      if (platform?.event === "message.created") {
        const text = platform.text || "";
        if (bubbleRole(platform.role) === "user" && text && !queuedTextsRef.current.has(text)) {
          appendUserBubble(text);
        }
      }

      const product = timelineProductFromEvent(event);
      if (!product) return;

      if (product.kind === "text_delta") {
        setPendingSince(null);
        appendAssistantChunk(product.text || "");
      } else if (product.kind === "thought_delta") {
        setPendingSince(null);
        appendThoughtChunk(product.text || "");
      } else if (product.kind === "user_message") {
        const text = product.text || "";
        if (text && !queuedTextsRef.current.has(text)) appendUserBubble(text);
      } else if (product.kind === "tool_call") {
        setPendingSince(null);
        upsertToolCall(product);
      } else if (product.kind === "tool_result") {
        applyToolUpdate(product);
      }
    },
    [appendAssistantChunk, appendThoughtChunk, appendUserBubble, upsertToolCall, applyToolUpdate, sealOnStop, onRunsChanged],
  );

  const refreshApprovals = useCallback(async () => {
    try {
      const list = (await api<Approval[]>(`/api/v1/runs/${runId}/approvals`)) || [];
      for (const ap of list) {
        if (ap.status === "pending" && !seenApprovals.current.has(ap.id)) {
          seenApprovals.current.add(ap.id);
          setItems((prev) => [...prev, { kind: "approval", id: nextId.current++, approval: ap }]);
        }
      }
    } catch (err) {
      console.warn(err);
    }
  }, [runId]);

  useEffect(() => {
    let stopped = false;
    const timers: {
      poll?: ReturnType<typeof setInterval>;
      approval?: ReturnType<typeof setInterval>;
      status?: ReturnType<typeof setInterval>;
    } = {};
    nextId.current = 1;
    afterSeq.current = 0;
    seenApprovals.current = new Set();
    hasAssistantTail.current = false;
    lastKnownTitle.current = null;
    setItems([]);
    setHistoryReady(false);
    setEditingTitle(false);
    setRun(null);

    (async () => {
      try {
        const r = await api<Run>(`/api/v1/runs/${runId}`);
        if (stopped) return;
        if (r.title) lastKnownTitle.current = r.title;
        setRun(r);
        const allEvents: RunEvent[] = [];
        let cursor = 0;
        for (;;) {
          const page = await api<{ events?: RunEvent[]; nextSeq?: number }>(
            `/api/v1/runs/${runId}/events?afterSeq=${cursor}`,
          );
          if (stopped) return;
          const batch = page.events || [];
          if (!batch.length) {
            afterSeq.current = page.nextSeq ?? cursor;
            break;
          }
          allEvents.push(...batch);
          cursor = page.nextSeq ?? batch[batch.length - 1]!.seq;
          afterSeq.current = cursor;
          if (batch.length < 500) break;
        }

        const draft = buildTimelineFromEvents(allEvents);
        let statusPatch: Partial<Run> | null = null;
        for (const e of allEvents) {
          const platform = platformEventFromPayload(e.payload);
          if (platform?.event === "run.status" && platform.status) {
            statusPatch = {
              ...(statusPatch || {}),
              status: platform.status,
              ...(platform.headSha ? { headSha: platform.headSha } : {}),
            };
          }
          if (platform?.event === "run.title" && platform.title) {
            lastKnownTitle.current = platform.title;
            statusPatch = { ...(statusPatch || {}), title: platform.title };
          }
        }
        if (statusPatch) {
          setRun((prev) => (prev ? { ...prev, ...statusPatch } : prev));
        }

        const msgs = (await api<RunMessage[]>(`/api/v1/runs/${runId}/messages`)) || [];
        if (stopped) return;
        setRunMessages(msgs);
        if (!draft.items.some((it) => it.kind === "bubble" && it.role === "user")) {
          const users = msgs.filter((m) => bubbleRole(m.role) === "user");
          if (users.length) {
            draft.items = [
              ...users.map((m) => ({
                kind: "bubble" as const,
                id: draft.nextId++,
                role: "user" as const,
                text: m.content,
              })),
              ...draft.items,
            ];
          }
        }

        nextId.current = draft.nextId;
        hasAssistantTail.current = draft.hasAssistantTail;
        setItems(draft.items);
        setHistoryReady(true);
        await refreshApprovals();
        onRunsChanged();

        if (stopped) return;
        timers.poll = setInterval(async () => {
          try {
            for (;;) {
              const live = await api<{ events?: RunEvent[]; nextSeq?: number }>(
                `/api/v1/runs/${runId}/events?afterSeq=${afterSeq.current}`,
              );
              const batch = live.events || [];
              if (!batch.length) {
                if (live.nextSeq != null) afterSeq.current = live.nextSeq;
                break;
              }
              for (const e of batch) {
                handleEvent(e);
                afterSeq.current = e.seq;
              }
              if (live.nextSeq != null) afterSeq.current = live.nextSeq;
              if (batch.length < 500) break;
            }
          } catch (err) {
            console.warn(err);
          }
        }, 1000);
        timers.approval = setInterval(refreshApprovals, 2000);
        timers.status = setInterval(async () => {
          try {
            const next = await api<Run>(`/api/v1/runs/${runId}`);
            if (stopped) return;
            const titleChanged = Boolean(next.title && next.title !== lastKnownTitle.current);
            if (next.title) lastKnownTitle.current = next.title;
            let statusChanged = false;
            setRun((prev) => {
              if (!prev) return next;
              statusChanged = prev.status !== next.status;
              if (
                prev.status === next.status &&
                prev.headSha === next.headSha &&
                prev.title === next.title
              ) {
                return prev;
              }
              return { ...prev, ...next };
            });
            if (isTerminalStatus(next.status)) sealOnStop();
            const ended = (next.status || "").toLowerCase();
            if (ended === "failed" || ended === "timed_out" || ended === "cancelled") {
              setPendingSince(null);
            }
            if (titleChanged || isSetupStatus(next.status) || statusChanged) {
              onRunsChanged();
            }
          } catch (err) {
            console.warn(err);
          }
        }, 2000);
        if (stopped) {
          if (timers.poll) clearInterval(timers.poll);
          if (timers.approval) clearInterval(timers.approval);
          if (timers.status) clearInterval(timers.status);
        }
      } catch (err) {
        if (!stopped) {
          setHistoryReady(true);
          toast(err instanceof Error ? err.message : String(err), "error");
        }
      }
    })();

    return () => {
      stopped = true;
      if (timers.poll) clearInterval(timers.poll);
      if (timers.approval) clearInterval(timers.approval);
      if (timers.status) clearInterval(timers.status);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [runId]);

  useEffect(() => {
    setPushDismissedHead(readSessionUi(runId).pushPromptDismissedHead ?? null);
    setPushPublished(false);
    setGitCompare(null);
    setRunPullRequests([]);
    setPromptQueue([]);
    queuedTextsRef.current.clear();
    setPendingSince(null);
  }, [runId]);

  useEffect(() => {
    if (!runId || !run) return;
    const status = (run.status || "").toLowerCase();
    const idle = ["completed", "waiting_for_user", "failed"].includes(status);
    if (!idle) return;

    let cancelled = false;
    (async () => {
      const [compare, prs] = await Promise.all([
        fetchGitCompare(runId),
        fetchRunPullRequests(runId),
      ]);
      if (cancelled) return;
      setGitCompare(compare);
      setRunPullRequests(prs);
      if (prs.some((pr) => pr.url)) setPushPublished(true);
    })().catch(() => undefined);

    return () => {
      cancelled = true;
    };
  }, [runId, run?.status, run?.headSha]);

  useEffect(() => {
    onMeta(run?.title || "Agent", run?.status);
    if (typeof document !== "undefined") {
      document.title = `${run?.title || "Agent"} · Zene`;
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [run?.title, run?.status]);

  useEffect(() => {
    const id = window.requestAnimationFrame(() => composerRef.current?.focus());
    return () => window.cancelAnimationFrame(id);
  }, [runId]);

  useEffect(() => {
    setSelectedModel(loadSelectedModel());
    api<LlmSettingsView>("/api/v1/settings/llm")
      .then(setLlmSettings)
      .catch(() => setLlmSettings(null));
  }, []);

  const pickerModels = useMemo(() => modelsForPicker(llmSettings), [llmSettings]);
  const hasLiveMeta = useMemo(() => timelineHasLiveMeta(items), [items]);
  const phase = sessionPhase(run?.status, hasLiveMeta, pendingSince != null);
  const chrome = composerChrome(phase);
  const repoName = run ? repoLabel(repos, run.repositoryId) : "";
  const headBranch = run?.headBranch || gitCompare?.head || "";
  const setupCopy = phase === "setup" ? setupStatusCopy(run?.status || "", repoName) : null;
  const pushHeadKey = run?.headSha || gitCompare?.head || "";

  const flushPromptQueue = useCallback(() => {
    setPromptQueue((prev) => {
      if (!prev.length) return prev;
      for (const item of prev) {
        appendUserBubble(item.text);
        queuedTextsRef.current.delete(item.text);
      }
      return [];
    });
  }, [appendUserBubble]);

  useEffect(() => {
    if (chrome.queueFollowUp) return;
    flushPromptQueue();
  }, [chrome.queueFollowUp, flushPromptQueue]);

  const showPushPrompt =
    historyReady &&
    !pushPublished &&
    hasUnpublishedChanges(gitCompare, runPullRequests) &&
    pushDismissedHead !== pushHeadKey;

  const dismissPushPrompt = useCallback(() => {
    const head = pushHeadKey || "dismissed";
    writeSessionUi(runId, { pushPromptDismissedHead: head });
    setPushDismissedHead(head);
  }, [runId, pushHeadKey]);

  const activePullRequest = useMemo(
    () => runPullRequests.find(isActivePullRequest) ?? null,
    [runPullRequests],
  );

  const onPushPublished = useCallback(
    (result: PublishResult) => {
      setPushPublished(true);
      if (result.pullRequest) {
        setRunPullRequests((prev) => {
          const rest = prev.filter((p) => p.id !== result.pullRequest?.id);
          return [result.pullRequest!, ...rest];
        });
      } else {
        void fetchRunPullRequests(runId).then(setRunPullRequests).catch(() => undefined);
      }
      onRunsChanged();
    },
    [runId, onRunsChanged],
  );

  const commitTitleEdit = useCallback(async () => {
    const next = titleDraft.trim();
    setEditingTitle(false);
    if (!next || !onRename || next === (run?.title || "")) return;
    try {
      await onRename(next);
      setRun((prev) => (prev ? { ...prev, title: next } : prev));
      lastKnownTitle.current = next;
    } catch {
      /* toast handled by parent */
    }
  }, [titleDraft, onRename, run?.title]);

  const retryRun = useCallback(async () => {
    const text = followUp.trim();
    setRetrying(true);
    setPendingSince(Date.now());
    try {
      if (text) {
        setItems((prev) => sealOpenMeta(prev));
        hasAssistantTail.current = false;
        appendBubble("user", text);
        setFollowUp("");
      }
      const r = await api<Run>(`/api/v1/runs/${runId}/retry`, {
        method: "POST",
        body: JSON.stringify(text ? { text } : {}),
      });
      setRun(r);
      onRunsChanged();
    } catch (err) {
      setPendingSince(null);
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setRetrying(false);
    }
  }, [followUp, runId, appendBubble, toast, onRunsChanged]);

  const sendFollowUp = useCallback(async () => {
    const text = followUp.trim();
    if (!text) return;
    if (chrome.submitVia === "retry") {
      await retryRun();
      return;
    }
    const queue = chrome.queueFollowUp;
    setSending(true);
    setPendingSince(Date.now());
    try {
      setItems((prev) => sealOpenMeta(prev));
      hasAssistantTail.current = false;
      if (queue) {
        queuedTextsRef.current.add(text);
        setPromptQueue((prev) => [...prev, { id: crypto.randomUUID(), text }]);
      } else {
        appendBubble("user", text);
      }
      setFollowUp("");
      await api(`/api/v1/runs/${runId}/messages`, {
        method: "POST",
        body: JSON.stringify({ text, clientMessageId: crypto.randomUUID() }),
      });
      if (queue) {
        setRun((prev) => (prev ? { ...prev, status: "running" } : prev));
      }
    } catch (err) {
      setPendingSince(null);
      queuedTextsRef.current.delete(text);
      setPromptQueue((prev) => prev.filter((item) => item.text !== text));
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setSending(false);
    }
  }, [followUp, runId, appendBubble, toast, chrome.submitVia, chrome.queueFollowUp, retryRun]);

  const cancelRun = useCallback(async () => {
    setCancelling(true);
    setPendingSince(null);
    sealOnStop();
    setRun((prev) => (prev ? { ...prev, status: "stopping" } : prev));
    try {
      const r = await api<Run>(`/api/v1/runs/${runId}/cancel`, { method: "POST", body: "{}" });
      setRun(r);
      sealOnStop();
      onRunsChanged();
      toast("Run stopped", "ok");
    } catch (err) {
      toast(err instanceof Error ? err.message : String(err), "error");
    } finally {
      setCancelling(false);
    }
  }, [runId, onRunsChanged, toast, sealOnStop]);

  const decideApproval = useCallback(
    async (itemId: number, approvalId: string, decision: ApprovalDecision) => {
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

  const forkTurn = useCallback(
    async (turnIndex: number) => {
      if (!run || !onRunStarted) return;
      const turns = buildConversationTurns(items, runMessages);
      const turn = turns[turnIndex];
      if (!turn?.assistantText.trim()) return;
      setForkingTurn(turnIndex);
      try {
        const prompt = buildForkPrompt(turns, turnIndex);
        const newRun = await api<Run>("/api/v1/runs", {
          method: "POST",
          body: JSON.stringify({
            repositoryId: run.repositoryId,
            prompt,
            baseRef: run.baseRef,
            model: run.model || selectedModel,
            permissionMode: run.permissionMode || "default",
            maxTurns: run.maxTurns ?? 100,
          }),
        });
        onRunStarted(newRun.id);
        onRunsChanged();
        toast("Forked to new agent", "ok");
      } catch (err) {
        toast(err instanceof Error ? err.message : String(err), "error");
      } finally {
        setForkingTurn(null);
      }
    },
    [run, onRunStarted, items, runMessages, selectedModel, onRunsChanged, toast],
  );

  return (
    <div
      className={[
        "relative grid h-full min-h-0 grid-cols-1",
        codePanelOpen
          ? [
              "grid-rows-[minmax(0,1fr)_minmax(0,38vh)] min-[981px]:grid-rows-1",
              sidebarCollapsed
                ? "min-[981px]:grid-cols-[calc(720px+2rem)_minmax(0,1fr)]"
                : "min-[981px]:grid-cols-[minmax(0,1fr)_var(--code-panel-w)]",
            ].join(" ")
          : "grid-rows-1",
      ].join(" ")}
      style={{ ["--code-panel-w" as string]: `${codeWidth}px` } as CSSProperties}
    >
      <div className="grid min-h-0 min-w-0 grid-rows-[36px_minmax(0,1fr)_auto] overflow-hidden bg-canvas-bg">
        <SessionHeader
          title={run?.title || "Agent"}
          repoName={repoName}
          headBranch={headBranch}
          model={run?.model || selectedModel}
          editingTitle={editingTitle}
          titleDraft={titleDraft}
          onTitleDraftChange={setTitleDraft}
          onStartEdit={
            onRename
              ? () => {
                  setTitleDraft(run?.title || "");
                  setEditingTitle(true);
                }
              : undefined
          }
          onCommitEdit={() => void commitTitleEdit()}
          onCancelEdit={() => setEditingTitle(false)}
          sidebarCollapsed={sidebarCollapsed}
          onOpenMenu={onOpenMenu}
          codePanelOpen={codePanelOpen}
          onToggleCodePanel={onToggleCodePanel}
        />
        <ChatTimeline
          runId={runId}
          items={items}
          historyReady={historyReady}
          setupCopy={setupCopy}
          pendingSince={pendingSince}
          assistantLive={phase === "live" || phase === "approval"}
          runMessages={runMessages}
          forkingTurn={forkingTurn}
          onFork={onRunStarted ? forkTurn : undefined}
          onToggleItem={toggleItem}
          onDecideApproval={decideApproval}
        />
        {activePullRequest ? (
          <div className="mx-auto w-full max-w-[720px] px-3.5 pb-2">
            <PullRequestCard
              runId={runId}
              pullRequest={activePullRequest}
              onUpdated={(pr) =>
                setRunPullRequests((prev) => prev.map((item) => (item.id === pr.id ? pr : item)))
              }
            />
          </div>
        ) : showPushPrompt && gitCompare ? (
          <div className="mx-auto w-full max-w-[720px] px-3.5 pb-2">
            <PushPromptCard
              runId={runId}
              title={run?.title}
              baseRef={run?.baseRef}
              headBranch={run?.headBranch}
              compare={gitCompare}
              onPublished={onPushPublished}
              onDismiss={dismissPushPrompt}
            />
          </div>
        ) : null}
        <div className="bg-canvas-bg px-4 pb-3 pt-1">
          <div className="mx-auto w-full max-w-[720px] px-3.5">
            <Composer
              ref={composerRef}
              value={followUp}
              onChange={setFollowUp}
              onSubmit={() => void sendFollowUp()}
              onStop={() => void cancelRun()}
              chrome={chrome}
              selectedModel={selectedModel}
              onSelectModel={(m) => {
                setSelectedModel(m);
                saveSelectedModel(m);
              }}
              models={pickerModels}
              queue={promptQueue}
              onRemoveQueueItem={(id) => {
                const item = promptQueue.find((p) => p.id === id);
                if (item) queuedTextsRef.current.delete(item.text);
                setPromptQueue((prev) => prev.filter((p) => p.id !== id));
              }}
              submitDisabled={!historyReady}
              submitBusy={sending || retrying}
              stopBusy={cancelling}
              leading={
                <button
                  type="button"
                  className="inline-flex h-6 w-6 items-center justify-center rounded-sm bg-chip text-muted hover:bg-line-strong hover:text-ink"
                  title="Add"
                  aria-label="Add"
                  onClick={() => toast("Attachments coming soon", "ok")}
                >
                  <IconPlus className="h-3.5 w-3.5" />
                </button>
              }
            />
          </div>
        </div>
      </div>

      {codePanelOpen && (
        <CodePanel
          runId={runId}
          defaultPrTitle={run?.title}
          defaultBaseRef={run?.baseRef}
          headBranch={run?.headBranch}
          gitCompare={gitCompare}
          width={codeWidth}
          onWidthChange={setCodeWidth}
          onCollapse={onToggleCodePanel}
          equalSplit={sidebarCollapsed}
        />
      )}
    </div>
  );
}
