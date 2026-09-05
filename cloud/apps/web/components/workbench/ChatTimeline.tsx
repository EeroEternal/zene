"use client";

import hljs from "highlight.js/lib/core";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  IconAlertCircle,
  IconChevronDown,
  IconChevronRight,
  IconLoader,
  IconRefresh,
  IconSkills,
} from "@/lib/icons";
import {
  allowsDeny,
  allowsOnce,
  approvalCardBody,
  extraDecisions,
  isAskUserApproval,
  matchAskUserApproval,
  parseAskUser,
} from "@/lib/approval";
import { isBusyStatus, liveThoughtPhaseCopy, waitingTurnCopy } from "@/lib/sessionPhase";
import type { Approval, ApprovalDecision, RunMessage } from "@/lib/types";
import {
  activitySummary,
  clusterActivityItems,
  formatElapsed,
  groupTimeline,
  isAskUserTool,
  thoughtBunchSummary,
  thoughtDurationMs,
  toolCommand,
  toolLabel,
  toolPath,
  type TimelineItem,
  type ThoughtItem,
  type ToolItem,
} from "@/lib/timeline";

function HighlightedPre({ text, className }: { text: string; className?: string }) {
  const html = useMemo(() => {
    if (!text) return "";
    try {
      if (text.length < 100_000) {
        return hljs.highlightAuto(text).value;
      }
    } catch { /* fall through */ }
    return text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
  }, [text]);
  return (
    <pre className={`code-viewer ${className || ""}`}>
      <code className="hljs" dangerouslySetInnerHTML={{ __html: html }} />
    </pre>
  );
}

function timelineScrollSig(items: TimelineItem[]): string {
  return items
    .map((it) => {
      if (it.kind === "bubble" || it.kind === "thought") {
        return `${it.id}:${it.text.length}:${it.kind === "thought" ? Number(it.sealed) : ""}`;
      }
      if (it.kind === "tool") return `${it.id}:${it.status}:${it.output?.length || 0}`;
      return `${it.id}:${it.kind}`;
    })
    .join("|");
}

function lastUserBubbleId(items: TimelineItem[]): number | null {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "bubble" && it.role === "user") return it.id;
  }
  return null;
}

function lastTurnIsWaitingForReply(items: TimelineItem[]): boolean {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "bubble" && it.role === "assistant") return false;
    if (it.kind === "bubble" && it.role === "user") return true;
  }
  return false;
}

function lastThoughtText(items: { kind: string; text?: string }[]): string {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "thought" && it.text) return it.text;
  }
  return "";
}

function ThoughtLivePreview({
  text,
  nested,
  phaseHint,
}: {
  text: string;
  nested: boolean;
  phaseHint?: string;
}) {
  const scrollerRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
  }, [text]);
  if (!text && !phaseHint) return null;
  return (
    <div className="mt-1 flex flex-col gap-1">
      {text ? (
        <div
          ref={scrollerRef}
          className={
            nested
              ? "max-h-[10.85em] overflow-hidden rounded-md border border-line bg-tertiary px-2.5 py-1 text-[13px] leading-[1.55] text-muted"
              : "max-h-[10.85em] overflow-hidden text-[13px] leading-[1.55] text-muted"
          }
        >
          <div className="whitespace-pre-wrap break-words [overflow-wrap:anywhere]">{text}</div>
        </div>
      ) : null}
      {phaseHint && (
        <div className="flex items-center gap-1.5 px-1 py-0.5 text-[11.5px] text-muted transition-opacity duration-300">
          <span className="relative flex h-2 w-2 shrink-0">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-primary opacity-60" />
            <span className="relative inline-flex h-2 w-2 rounded-full bg-primary" />
          </span>
          <span className="truncate font-medium text-ink/75">{phaseHint}</span>
        </div>
      )}
    </div>
  );
}

/** Leave room to park the latest user bubble centered in the viewport while a turn is open. */
function lastTurnNeedsSpacer(
  items: TimelineItem[],
  pendingSince: number | null,
  assistantLive: boolean,
): boolean {
  if (pendingSince != null || assistantLive) return true;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it.kind === "bubble" && it.role === "user") return true;
    if (it.kind === "bubble" && it.role === "assistant") return false;
  }
  return false;
}
import { buildConversationTurns, turnIndexByEndItemId } from "@/lib/turnActions";
import { Markdown } from "../Markdown";
import { TurnActions } from "../TurnActions";

export type ApprovalChoice = {
  optionId?: string;
  answer?: string;
};

function pairAskUserApprovals(items: TimelineItem[]) {
  const byToolId = new Map<number, { approval: Approval; itemId: number }>();
  const pairedApprovalItemIds = new Set<number>();
  const used = new Set<string>();
  const approvals = items
    .filter((it): it is Extract<TimelineItem, { kind: "approval" }> => it.kind === "approval")
    .filter((it) => isAskUserApproval(it.approval));
  for (const item of items) {
    if (item.kind !== "tool" || !isAskUserTool(item)) continue;
    const hit = matchAskUserApproval(
      item.input,
      approvals.map((row) => row.approval),
      used,
    );
    if (!hit) continue;
    used.add(hit.id);
    const row = approvals.find((ap) => ap.approval.id === hit.id);
    if (!row) continue;
    byToolId.set(item.id, { approval: hit, itemId: row.id });
    pairedApprovalItemIds.add(row.id);
  }
  return { byToolId, pairedApprovalItemIds };
}

function AskUserCard({
  itemId,
  questionSource,
  output,
  approval,
  approvalItemId,
  decided,
  onDecide,
}: {
  itemId: number;
  questionSource: unknown;
  output?: string;
  approval?: Approval;
  approvalItemId?: number;
  decided: boolean;
  onDecide: (
    itemId: number,
    approvalId: string,
    decision: ApprovalDecision,
    extra?: ApprovalChoice,
  ) => void;
}) {
  const prompt = parseAskUser(questionSource) || (approval ? parseAskUser(approval.payload) : null);
  const question = prompt?.question || "The agent asked a question";
  const options = prompt?.options || [];
  const [draft, setDraft] = useState("");
  const answered = decided || Boolean(output?.trim());
  const answer = (output || "").trim();
  const targetItemId = approvalItemId ?? itemId;

  const send = (extra: ApprovalChoice) => {
    if (!approval) return;
    onDecide(targetItemId, approval.id, "allow-once", extra);
  };

  return (
    <div className="self-stretch rounded-md border-l-2 border-primary bg-tint p-3.5 min-[981px]:ml-9">
      <h4 className="mb-2 text-[13px] font-semibold text-ink">{question}</h4>
      {answered ? (
        answer ? (
          <div className="text-[13px] leading-[1.5] text-muted [overflow-wrap:anywhere]">
            {answer}
          </div>
        ) : (
          <div className="text-[12px] text-muted">Answered</div>
        )
      ) : (
        <div className="space-y-2">
          {options.length > 0 && (
            <div className="flex flex-col gap-1.5">
              {options.map((opt) => (
                <button
                  key={opt.id}
                  type="button"
                  className="btn btn-sm h-auto min-h-7 w-full justify-start whitespace-normal py-1.5 text-left"
                  disabled={!approval}
                  onClick={() => send({ optionId: opt.id })}
                >
                  <span className="font-medium">{opt.label}</span>
                  {opt.description ? (
                    <span className="ml-2 font-normal text-muted">{opt.description}</span>
                  ) : null}
                </button>
              ))}
            </div>
          )}
          {approval ? (
            <div className="flex gap-2">
              <input
                className="field-input min-h-7 py-1.5 text-[13px]"
                value={draft}
                placeholder="Type an answer"
                onChange={(event) => setDraft(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && draft.trim()) {
                    event.preventDefault();
                    send({ optionId: "free-text", answer: draft.trim() });
                  }
                }}
              />
              <button
                type="button"
                className="btn btn-primary btn-sm shrink-0"
                disabled={!draft.trim()}
                onClick={() => send({ optionId: "free-text", answer: draft.trim() })}
              >
                Send
              </button>
            </div>
          ) : (
            <p className="text-[12px] text-muted">
              This question is no longer waiting. Reply in the follow-up box below.
            </p>
          )}
        </div>
      )}
    </div>
  );
}

interface ChatTimelineProps {
  runId: string;
  items: TimelineItem[];
  historyReady: boolean;
  setupCopy?: { title: string; detail: string } | null;
  /** Epoch ms when the user sent a turn that has not produced activity yet. */
  pendingSince?: number | null;
  runStatus?: string | null;
  lastError?: string | null;
  onRetry?: () => void;
  retrying?: boolean;
  assistantLive: boolean;
  runMessages: RunMessage[];
  forkingTurn: number | null;
  onFork?: (turnIndex: number) => void;
  onToggleItem: (id: number) => void;
  onDecideApproval: (
    itemId: number,
    approvalId: string,
    decision: ApprovalDecision,
    extra?: { optionId?: string; answer?: string },
  ) => void;
}

export function ChatTimeline({
  runId,
  items,
  historyReady,
  setupCopy,
  pendingSince = null,
  runStatus = null,
  lastError = null,
  onRetry,
  retrying = false,
  assistantLive,
  runMessages,
  forkingTurn,
  onFork,
  onToggleItem,
  onDecideApproval,
}: ChatTimelineProps) {
  const [openGroups, setOpenGroups] = useState<Set<string>>(() => new Set());
  const [openBunches, setOpenBunches] = useState<Set<string>>(() => new Set());
  const [fullLogItems, setFullLogItems] = useState<Set<number>>(() => new Set());
  const [nowTick, setNowTick] = useState(() => Date.now());
  const stickToBottom = useRef(true);
  const followingTurn = useRef(false);
  const lastCenteredUserId = useRef<number | null>(null);
  const justSentId = useRef<number | null>(null);
  const messagesRef = useRef<HTMLDivElement>(null);
  const innerRef = useRef<HTMLDivElement>(null);
  const spacerRef = useRef<HTMLDivElement>(null);
  const [spacerPx, setSpacerPx] = useState(0);

  const toggleFullLog = useCallback((id: number) => {
    setFullLogItems((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  const segments = useMemo(() => groupTimeline(items), [items]);
  const askUserPairs = useMemo(() => pairAskUserApprovals(items), [items]);
  const hasLiveMeta = useMemo(
    () => segments.some((s) => s.type === "activity" && s.live),
    [segments],
  );
  const waitingForReply =
    historyReady &&
    !hasLiveMeta &&
    !setupCopy &&
    lastTurnIsWaitingForReply(items) &&
    (pendingSince != null ||
      assistantLive ||
      isBusyStatus(runStatus) ||
      (runStatus || "").toLowerCase() === "created");
  const waitOriginRef = useRef<number | null>(null);
  if (waitingForReply) {
    if (waitOriginRef.current == null) {
      waitOriginRef.current = pendingSince ?? Date.now();
    }
  } else {
    waitOriginRef.current = null;
  }
  const waitCopy =
    waitingForReply && waitOriginRef.current != null
      ? waitingTurnCopy(nowTick - waitOriginRef.current, runStatus)
      : null;
  const conversationTurns = useMemo(
    () => buildConversationTurns(items, runMessages),
    [items, runMessages],
  );
  const turnEndByItemId = useMemo(() => turnIndexByEndItemId(items), [items]);
  const lastTurnIndex = conversationTurns.length - 1;

  const scrollToBottom = useCallback((force = false) => {
    requestAnimationFrame(() => {
      const el = messagesRef.current;
      if (!el) return;
      if (!force && !stickToBottom.current) return;
      el.scrollTop = el.scrollHeight;
    });
  }, []);

  const scrollUserBubbleToReading = useCallback((bubbleId: number) => {
    const container = messagesRef.current;
    if (!container) return;
    const bubble = container.querySelector<HTMLElement>(`[data-bubble-id="${bubbleId}"]`);
    if (!bubble) return;
    const bubbleTop =
      bubble.getBoundingClientRect().top -
      container.getBoundingClientRect().top +
      container.scrollTop;
    const targetOffset = Math.max(16, Math.floor((container.clientHeight - bubble.offsetHeight) / 2));
    container.scrollTo({
      top: Math.max(0, bubbleTop - targetOffset),
      behavior: "smooth",
    });
  }, []);

  const updateSpacer = useCallback(() => {
    const container = messagesRef.current;
    const spacer = spacerRef.current;
    if (!container) return;
    if (!lastTurnNeedsSpacer(items, pendingSince, assistantLive)) {
      setSpacerPx((prev) => (prev === 0 ? prev : 0));
      return;
    }
    const userId = lastUserBubbleId(items);
    const bubble =
      userId != null
        ? container.querySelector<HTMLElement>(`[data-bubble-id="${userId}"]`)
        : null;
    if (!bubble || !spacer) {
      setSpacerPx((prev) => (prev === 0 ? prev : 0));
      return;
    }
    const following = spacer.offsetTop - bubble.offsetTop;
    const targetOffset = Math.max(16, Math.floor((container.clientHeight - bubble.offsetHeight) / 2));
    const next = Math.max(0, Math.floor(container.clientHeight - targetOffset - following));
    setSpacerPx((prev) => (Math.abs(prev - next) < 2 ? prev : next));
  }, [items, pendingSince, assistantLive]);

  useEffect(() => {
    setOpenGroups(new Set());
    setOpenBunches(new Set());
    stickToBottom.current = true;
    followingTurn.current = false;
    lastCenteredUserId.current = null;
    justSentId.current = null;
    setSpacerPx(0);
  }, [runId]);

  useEffect(() => {
    if (!hasLiveMeta && pendingSince == null && !waitingForReply) return;
    const id = window.setInterval(() => setNowTick(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [hasLiveMeta, pendingSince, waitingForReply]);

  const scrollSig = useMemo(() => timelineScrollSig(items), [items]);
  const latestUserBubbleId = useMemo(() => lastUserBubbleId(items), [items]);

  useLayoutEffect(() => {
    updateSpacer();
  }, [updateSpacer, scrollSig, pendingSince, hasLiveMeta, spacerPx]);

  useEffect(() => {
    const el = messagesRef.current;
    if (!el || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => updateSpacer());
    ro.observe(el);
    return () => ro.disconnect();
  }, [updateSpacer]);

  useEffect(() => {
    if (!historyReady) return;
    scrollToBottom(true);
    lastCenteredUserId.current = lastUserBubbleId(items);
    justSentId.current = null;
    followingTurn.current = false;
    // Only on first history load / session switch — a new send must park, not jump to bottom.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [historyReady, runId, scrollToBottom]);

  useLayoutEffect(() => {
    if (!historyReady) return;
    if (
      latestUserBubbleId != null &&
      latestUserBubbleId !== lastCenteredUserId.current
    ) {
      lastCenteredUserId.current = latestUserBubbleId;
      justSentId.current = latestUserBubbleId;
      stickToBottom.current = true;
      followingTurn.current = true;
    }
    if (justSentId.current != null) {
      scrollUserBubbleToReading(justSentId.current);
      if (spacerPx > 0) justSentId.current = null;
    }
  }, [historyReady, latestUserBubbleId, spacerPx, scrollUserBubbleToReading]);

  useEffect(() => {
    if (!historyReady) return;
    if (justSentId.current != null) return;
    if ((stickToBottom.current || followingTurn.current) && spacerPx === 0) {
      scrollToBottom();
    }
  }, [scrollSig, latestUserBubbleId, historyReady, pendingSince, spacerPx, scrollToBottom]);

  useEffect(() => {
    if (!historyReady) return;
    const normalized = (runStatus || "").toLowerCase();
    if (normalized === "failed" || normalized === "timed_out") {
      justSentId.current = null;
      scrollToBottom(true);
    }
  }, [runStatus, historyReady, scrollToBottom]);

  const toggleGroup = useCallback((key: string) => {
    setOpenGroups((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const toggleBunch = useCallback((key: string) => {
    setOpenBunches((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  }, []);

  const renderThoughtRow = (item: ThoughtItem, nested: boolean) => {
    const durationMs = thoughtDurationMs(item, nowTick);
    const elapsed = formatElapsed(durationMs);
    const live = !item.sealed;
    const thoughtLabel = live ? `Thinking · ${elapsed}` : `Thought for ${elapsed}`;
    const preview = live && !item.expanded;
    const showFull = item.expanded && Boolean(item.text);
    const phaseHint = live ? liveThoughtPhaseCopy(durationMs) : undefined;
    return (
      <div key={item.id}>
        <button
          type="button"
          className={
            nested
              ? "flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
              : "relative flex w-full items-center gap-1.5 rounded-md py-1 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
          }
          aria-expanded={item.expanded || live}
          onClick={() => onToggleItem(item.id)}
        >
          {nested ? (
            item.expanded || live ? (
              <IconChevronDown className="h-3 w-3 shrink-0" />
            ) : (
              <IconChevronRight className="h-3 w-3 shrink-0" />
            )
          ) : (
            <span className="pointer-events-none absolute right-full top-1/2 mr-1 -translate-y-1/2 text-placeholder">
              {item.expanded || live ? (
                <IconChevronDown className="h-3.5 w-3.5" />
              ) : (
                <IconChevronRight className="h-3.5 w-3.5" />
              )}
            </span>
          )}
          {nested ? <IconSkills className="h-3 w-3 shrink-0" /> : null}
          <span className="min-w-0 flex-1 truncate font-medium">{thoughtLabel}</span>
          {live && (
            <IconLoader className="h-3 w-3 shrink-0 animate-spin text-primary" />
          )}
        </button>
        {preview && item.text ? (
          <ThoughtLivePreview text={item.text} nested={nested} phaseHint={phaseHint} />
        ) : null}
        {showFull ? (
          <div
            className={
              nested
                ? "mt-0.5 whitespace-pre-wrap break-words rounded-md border border-line bg-tertiary px-2.5 py-1.5 text-[12px] leading-[1.5] text-muted [overflow-wrap:anywhere]"
                : "mt-0.5 whitespace-pre-wrap break-words text-[12px] leading-[1.5] text-muted [overflow-wrap:anywhere]"
            }
          >
            {item.text}
          </div>
        ) : null}
      </div>
    );
  };

  const turnEndActions = (itemId: number) => {
    const turnIndex = turnEndByItemId.get(itemId);
    if (turnIndex == null) return null;
    const turn = conversationTurns[turnIndex];
    if (!turn) return null;
    return (
      <TurnActions
        runId={runId}
        turn={turn}
        visible={!(assistantLive && turnIndex === lastTurnIndex)}
        forking={forkingTurn === turn.index}
        onFork={onFork ? () => onFork(turn.index) : undefined}
      />
    );
  };

  return (
    <div
      ref={messagesRef}
      className="flex flex-col gap-3 overflow-y-auto no-scrollbar px-4 pb-2 pt-1"
      onScroll={() => {
        const el = messagesRef.current;
        if (!el) return;
        const gap = el.scrollHeight - el.scrollTop - el.clientHeight;
        if (followingTurn.current && latestUserBubbleId != null) {
          const bubble = el.querySelector<HTMLElement>(`[data-bubble-id="${latestUserBubbleId}"]`);
          if (bubble) {
            const top =
              bubble.getBoundingClientRect().top - el.getBoundingClientRect().top;
            if (top < -48) followingTurn.current = false;
          }
        }
        stickToBottom.current = followingTurn.current || gap < 80;
      }}
    >
      <div ref={innerRef} className="mx-auto flex w-full max-w-[720px] flex-col gap-3 px-3.5">
        {setupCopy && (
          <div
            className="-mx-3.5 flex items-start gap-3 self-stretch rounded-md bg-canvas px-3.5 py-3 shadow-card"
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
        {!historyReady && (
          <div className="flex items-center gap-2 py-6 text-[12.5px] text-muted" role="status">
            <IconLoader className="h-3.5 w-3.5 shrink-0 animate-spin" />
            Loading conversation…
          </div>
        )}
        {historyReady &&
          segments.map((seg) => {
            if (seg.type === "activity") {
              const only = seg.items.length === 1 ? seg.items[0] : null;
              if (only?.kind === "thought") {
                return (
                  <div key={seg.key} className="self-stretch">
                    {renderThoughtRow(only, false)}
                    {turnEndActions(only.id)}
                  </div>
                );
              }
              if (only?.kind === "tool" && isAskUserTool(only)) {
                const paired = askUserPairs.byToolId.get(only.id);
                return (
                  <div key={seg.key} className="self-stretch">
                    <AskUserCard
                      itemId={only.id}
                      questionSource={only.input}
                      output={only.output}
                      approval={paired?.approval}
                      approvalItemId={paired?.itemId}
                      decided={Boolean(paired?.approval && items.some((it) => it.kind === "approval" && it.approval.id === paired.approval.id && it.decision))}
                      onDecide={onDecideApproval}
                    />
                    {turnEndActions(only.id)}
                  </div>
                );
              }
              const open = openGroups.has(seg.key);
              const summary = activitySummary(seg.items, nowTick);
              const liveThoughtPreview = !open && seg.live ? lastThoughtText(seg.items) : "";
              return (
                <div key={seg.key} className="self-stretch">
                  <button
                    type="button"
                    className="relative flex w-full items-center gap-1.5 rounded-md py-1 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                    aria-expanded={open}
                    onClick={() => toggleGroup(seg.key)}
                  >
                    <span className="pointer-events-none absolute right-full top-1/2 mr-1 -translate-y-1/2 text-placeholder">
                      {open ? (
                        <IconChevronDown className="h-3.5 w-3.5" />
                      ) : (
                        <IconChevronRight className="h-3.5 w-3.5" />
                      )}
                    </span>
                    <span className="min-w-0 flex-1 truncate font-medium text-muted">{summary}</span>
                    {seg.live ? (
                      <IconLoader className="h-3 w-3 shrink-0 animate-spin text-primary" />
                    ) : null}
                  </button>
                  {liveThoughtPreview ? (
                    (() => {
                      const lt = [...seg.items].reverse().find((t): t is ThoughtItem => t.kind === "thought" && !t.sealed);
                      const ltHint = lt ? liveThoughtPhaseCopy(thoughtDurationMs(lt, nowTick)) : undefined;
                      return <ThoughtLivePreview text={liveThoughtPreview} nested={false} phaseHint={ltHint} />;
                    })()
                  ) : null}
                  {open && (
                    <div className="mt-0.5 ml-1.5 space-y-0.5 border-l border-line pl-2.5">
                      {clusterActivityItems(seg.items, nowTick).map((row) => {
                        if (row.type === "thought") {
                          return renderThoughtRow(row.item, true);
                        }

                        if (row.type === "thought-bunch") {
                          const bunchOpen = openBunches.has(row.key);
                          const liveThought = row.items.some((t) => !t.sealed);
                          const liveText =
                            [...row.items].reverse().find((t) => !t.sealed)?.text ||
                            row.items[row.items.length - 1]?.text ||
                            "";
                          const liveItem = row.items.find((t) => !t.sealed);
                          const liveHint = liveItem ? liveThoughtPhaseCopy(thoughtDurationMs(liveItem, nowTick)) : undefined;
                          return (
                            <div key={row.key}>
                              <button
                                type="button"
                                className="flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                                aria-expanded={bunchOpen || liveThought}
                                onClick={() => toggleBunch(row.key)}
                              >
                                {bunchOpen || liveThought ? (
                                  <IconChevronDown className="h-3 w-3 shrink-0" />
                                ) : (
                                  <IconChevronRight className="h-3 w-3 shrink-0" />
                                )}
                                <IconSkills className="h-3 w-3 shrink-0" />
                                <span className="font-medium">
                                  {thoughtBunchSummary(row.items, nowTick)}
                                </span>
                                {liveThought && (
                                  <IconLoader className="ml-auto h-3 w-3 shrink-0 animate-spin" />
                                )}
                              </button>
                              {liveThought && !bunchOpen && liveText ? (
                                <ThoughtLivePreview text={liveText} nested phaseHint={liveHint} />
                              ) : null}
                              {bunchOpen && (
                                <div className="mt-0.5 space-y-1 rounded-md border border-line bg-tertiary px-2.5 py-1.5 text-[12px] leading-[1.5] text-muted [overflow-wrap:anywhere]">
                                  {row.items.map((t, idx) => (
                                    <div
                                      key={t.id}
                                      className={
                                        idx > 0
                                          ? "whitespace-pre-wrap break-words border-t border-line pt-1.5"
                                          : "whitespace-pre-wrap break-words"
                                      }
                                    >
                                      {t.text}
                                    </div>
                                  ))}
                                </div>
                              )}
                            </div>
                          );
                        }

                        const renderTool = (item: ToolItem) => {
                          const busy = /pending|in_progress|running/i.test(item.status);
                          const failed = /failed/i.test(item.status);
                          const label = toolLabel(item);
                          const command = toolCommand(item);
                          const path = toolPath(item);
                          const detailCmd = command
                            ? `$ ${command}`
                            : path
                              ? path
                              : item.input && !item.input.trimStart().startsWith("{")
                                ? item.input
                                : "";
                          const output = item.output || "";
                          const isLongOutput = output.split("\n").length > 30 || output.length > 2000;
                          const showFull = fullLogItems.has(item.id);
                          return (
                            <div key={item.id}>
                              <button
                                type="button"
                                className="flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                                aria-expanded={item.expanded}
                                onClick={() => onToggleItem(item.id)}
                              >
                                {item.expanded ? (
                                  <IconChevronDown className="h-3 w-3 shrink-0" />
                                ) : (
                                  <IconChevronRight className="h-3 w-3 shrink-0" />
                                )}
                                <span
                                  className={[
                                    "min-w-0 flex-1 truncate",
                                    failed ? "text-danger" : "text-ink",
                                  ].join(" ")}
                                >
                                  {label}
                                </span>
                                {busy ? (
                                  <IconLoader className="h-3 w-3 shrink-0 animate-spin text-primary" />
                                ) : failed ? (
                                  <span className="shrink-0 text-[10px] font-medium text-danger">
                                    failed
                                  </span>
                                ) : null}
                              </button>
                              {item.expanded && (
                                <div className="mt-0.5 space-y-1 rounded-md border border-line bg-tertiary px-2.5 py-1.5">
                                  {detailCmd && (
                                    <HighlightedPre
                                      text={detailCmd}
                                      className="m-0 max-h-36 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[11px] leading-[1.45] text-muted"
                                    />
                                  )}
                                  {output && (
                                    <div>
                                      <HighlightedPre
                                        text={output}
                                        className={[
                                          "m-0 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[11px] leading-[1.45]",
                                          showFull ? "max-h-[500px]" : "max-h-40",
                                          detailCmd ? "border-t border-line pt-1.5" : "",
                                          failed ? "text-danger" : "text-muted",
                                        ].join(" ")}
                                      />
                                      {isLongOutput && (
                                        <button
                                          type="button"
                                          className="mt-1 text-[10.5px] font-medium text-primary hover:underline"
                                          onClick={() => toggleFullLog(item.id)}
                                        >
                                          {showFull ? "Show less" : `Show full output (${output.split("\n").length} lines)`}
                                        </button>
                                      )}
                                    </div>
                                  )}
                                  {!detailCmd && !output && (
                                    <div className="text-[11px] text-placeholder">No details</div>
                                  )}
                                </div>
                              )}
                            </div>
                          );
                        };

                        if (row.type === "tool-bunch") {
                          const bunchOpen = openBunches.has(row.key);
                          const busy = row.items.some((t) =>
                            /pending|in_progress|running/i.test(t.status),
                          );
                          const failed = row.items.some((t) => /failed/i.test(t.status));
                          const paths = row.items
                            .map((t) => toolPath(t) || toolLabel(t))
                            .filter(Boolean);
                          return (
                            <div key={row.key}>
                              <button
                                type="button"
                                className="flex w-full items-center gap-1.5 rounded-md px-1 py-0.5 text-left text-[12px] text-muted hover:bg-secondary hover:text-ink"
                                aria-expanded={bunchOpen}
                                onClick={() => toggleBunch(row.key)}
                              >
                                {bunchOpen ? (
                                  <IconChevronDown className="h-3 w-3 shrink-0" />
                                ) : (
                                  <IconChevronRight className="h-3 w-3 shrink-0" />
                                )}
                                <span
                                  className={[
                                    "min-w-0 flex-1 truncate",
                                    failed ? "text-danger" : "text-ink",
                                  ].join(" ")}
                                >
                                  {row.summary}
                                </span>
                                {busy ? (
                                  <IconLoader className="h-3 w-3 shrink-0 animate-spin text-primary" />
                                ) : (
                                  <span className="shrink-0 text-[10px] tabular-nums text-placeholder">
                                    {row.items.length}
                                  </span>
                                )}
                              </button>
                              {bunchOpen && (
                                <div className="mt-0.5 ml-1.5 space-y-0.5 border-l border-line pl-2">
                                  {row.family === "edit" || row.family === "read" ? (
                                    paths.map((p, idx) => (
                                      <div
                                        key={`${row.key}-${idx}`}
                                        className="truncate px-1 py-0.5 text-[12px] text-muted"
                                        title={p}
                                      >
                                        {p}
                                      </div>
                                    ))
                                  ) : (
                                    row.items.map((t) => renderTool(t))
                                  )}
                                </div>
                              )}
                            </div>
                          );
                        }

                        return renderTool(row.item);
                      })}
                    </div>
                  )}
                  {turnEndActions(seg.items[seg.items.length - 1].id)}
                </div>
              );
            }

            const item = seg.item;
            if (item.kind === "tool" && isAskUserTool(item)) {
              const paired = askUserPairs.byToolId.get(item.id);
              return (
                <div key={item.id} className="self-stretch">
                  <AskUserCard
                    itemId={item.id}
                    questionSource={item.input}
                    output={item.output}
                    approval={paired?.approval}
                    approvalItemId={paired?.itemId}
                    decided={Boolean(
                      paired &&
                        items.some(
                          (it) =>
                            it.kind === "approval" &&
                            it.approval.id === paired.approval.id &&
                            it.decision,
                        ),
                    )}
                    onDecide={onDecideApproval}
                  />
                  {turnEndActions(item.id)}
                </div>
              );
            }
            if (item.kind === "approval") {
              if (askUserPairs.pairedApprovalItemIds.has(item.id)) {
                return null;
              }
              if (isAskUserApproval(item.approval)) {
                return (
                  <div key={item.id} className="self-stretch">
                    <AskUserCard
                      itemId={item.id}
                      questionSource={item.approval.payload}
                      approval={item.approval}
                      approvalItemId={item.id}
                      decided={Boolean(item.decision)}
                      onDecide={onDecideApproval}
                    />
                    {turnEndActions(item.id)}
                  </div>
                );
              }
              const ap = item.approval;
              const summary = approvalCardBody(ap.payload);
              const allowed = ap.allowedDecisions || ["allow-once", "reject-once"];
              const extra = extraDecisions(allowed);
              const decided = Boolean(item.decision);
              return (
                <div
                  key={item.id}
                  className={`self-stretch rounded-md border-l-2 border-warn bg-warn-soft p-3.5 min-[981px]:ml-9 ${decided ? "opacity-[.65]" : ""}`}
                >
                  <h4 className="mb-1.5 text-[13px] font-semibold text-warn-ink">
                    Permission · {ap.kind || "tool"} · {ap.risk || ""}
                    {item.decision ? ` · ${item.decision}` : ""}
                  </h4>
                  <div className="mb-3 max-h-40 overflow-y-auto whitespace-pre-wrap break-all rounded-md border border-line bg-canvas p-2.5 font-mono text-xs text-muted [overflow-wrap:anywhere]">
                    {summary}
                  </div>
                  <div className="flex flex-wrap gap-2">
                    {allowsOnce(allowed) && (
                      <button
                        type="button"
                        className="btn btn-primary btn-sm"
                        disabled={decided}
                        onClick={() => onDecideApproval(item.id, ap.id, "allow-once")}
                      >
                        Allow once
                      </button>
                    )}
                    {allowsDeny(allowed) && (
                      <button
                        type="button"
                        className="btn btn-danger btn-sm"
                        disabled={decided}
                        onClick={() => onDecideApproval(item.id, ap.id, "reject-once")}
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
                        onClick={() => onDecideApproval(item.id, ap.id, d)}
                      >
                        {d}
                      </button>
                    ))}
                  </div>
                  {turnEndActions(item.id)}
                </div>
              );
            }

            if (item.kind === "bubble" && item.role === "user") {
              return (
                <div
                  key={item.id}
                  data-bubble-id={item.id}
                  className="-mx-3.5 min-w-0 self-stretch whitespace-pre-wrap break-words rounded-md bg-tertiary px-3.5 py-2.5 text-[13.5px] leading-[1.55] text-ink [overflow-wrap:anywhere]"
                >
                  {item.text}
                </div>
              );
            }

            if (item.kind === "bubble") {
              return (
                <div
                  key={item.id}
                  className="min-w-0 w-full self-stretch text-[13.5px] leading-[1.55] text-ink"
                >
                  <Markdown text={item.text} />
                  {turnEndActions(item.id)}
                </div>
              );
            }

            return null;
          })}
        {waitCopy && (
          <div className="self-stretch" role="status" aria-live="polite">
            <div className="relative flex w-full items-center gap-1.5 py-1 text-[12px] text-muted">
              <span className="pointer-events-none absolute right-full top-1/2 mr-1 -translate-y-1/2 text-placeholder">
                <IconChevronDown className="h-3.5 w-3.5" />
              </span>
              <span className="min-w-0 flex-1 truncate font-medium">{waitCopy.title}</span>
              <IconLoader className="h-3 w-3 shrink-0 animate-spin text-primary" />
            </div>
            <p className="m-0 mt-0.5 text-[12.5px] leading-[1.45] text-muted">
              {waitCopy.detail}
            </p>
          </div>
        )}
        {((runStatus === "failed" || runStatus === "timed_out") ||
          (Boolean(lastError) && lastError !== "user_retry" && !isBusyStatus(runStatus))) && (
          <div
            className="self-stretch rounded-md border border-destructive/30 bg-destructive/5 p-3.5 text-foreground"
            role="alert"
          >
            <div className="flex items-start gap-2.5">
              <IconAlertCircle className="mt-0.5 h-4 w-4 shrink-0 text-destructive" />
              <div className="min-w-0 flex-1">
                <div className="text-[13px] font-semibold text-destructive">
                  {runStatus === "timed_out" ? "Execution timed out" : "Execution failed"}
                </div>
                {lastError && lastError !== "user_retry" ? (
                  <p className="mt-1 whitespace-pre-wrap break-words font-mono text-[12px] leading-relaxed text-muted-foreground">
                    {lastError}
                  </p>
                ) : (
                  <p className="mt-1 text-[12px] leading-relaxed text-muted-foreground">
                    {runStatus === "timed_out"
                      ? "The task took too long to complete. You can retry with a new turn."
                      : "The agent encountered an error while executing this task. You can retry or verify your model settings."}
                  </p>
                )}
                {onRetry && (
                  <div className="mt-3 flex items-center gap-2">
                    <button
                      type="button"
                      className="btn btn-sm inline-flex items-center gap-1.5 border-destructive/20 text-destructive hover:bg-destructive/10"
                      disabled={retrying}
                      onClick={onRetry}
                    >
                      <IconRefresh className={`h-3 w-3 ${retrying ? "animate-spin" : ""}`} />
                      {retrying ? "Retrying..." : "Retry"}
                    </button>
                  </div>
                )}
              </div>
            </div>
          </div>
        )}
        <div ref={spacerRef} aria-hidden className="pointer-events-none shrink-0" style={{ height: spacerPx }} />
      </div>
    </div>
  );
}
