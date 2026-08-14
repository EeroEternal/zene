"use client";

import { useCallback, useEffect, useState } from "react";
import { IconBranch, IconCheck, IconCopy, IconThumbsDown, IconThumbsUp } from "@/lib/icons";
import {
  formatRelativeTime,
  loadTurnRating,
  saveTurnRating,
  turnCopyText,
  type ConversationTurn,
  type TurnRating,
} from "@/lib/turnActions";
import { useToast } from "./Toast";

interface TurnActionsProps {
  runId: string;
  turn: ConversationTurn;
  /** Hide while the assistant response is still streaming. */
  visible: boolean;
  forking?: boolean;
  onFork?: () => void;
}

export function TurnActions({ runId, turn, visible, forking = false, onFork }: TurnActionsProps) {
  const toast = useToast();
  const [rating, setRating] = useState<TurnRating | null>(() => loadTurnRating(runId, turn.index));
  const [copied, setCopied] = useState(false);
  const relative = formatRelativeTime(turn.assistantAt);

  useEffect(() => {
    setRating(loadTurnRating(runId, turn.index));
  }, [runId, turn.index]);

  const copyTurn = useCallback(async () => {
    const text = turnCopyText(turn);
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      toast("Copied turn", "ok");
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      toast("Could not copy", "error");
    }
  }, [toast, turn]);

  const vote = useCallback(
    (next: TurnRating) => {
      const saved = saveTurnRating(runId, turn.index, next);
      setRating(saved);
    },
    [runId, turn.index],
  );

  if (!visible || !turn.assistantText.trim()) return null;

  return (
    <div className="mt-1 flex items-center gap-2 text-[11px] text-muted">
      {relative && <span className="shrink-0 tabular-nums">{relative}</span>}
      <div className="ml-auto flex items-center gap-0.5">
        <button
          type="button"
          className={[
            "inline-flex h-6 w-6 items-center justify-center rounded-md transition-colors",
            rating === "up"
              ? "bg-secondary text-ink"
              : "text-muted hover:bg-secondary hover:text-ink",
          ].join(" ")}
          title="Good response"
          aria-label="Good response"
          aria-pressed={rating === "up"}
          onClick={() => vote("up")}
        >
          <IconThumbsUp className="h-3.5 w-3.5" />
        </button>
        <button
          type="button"
          className={[
            "inline-flex h-6 w-6 items-center justify-center rounded-md transition-colors",
            rating === "down"
              ? "bg-secondary text-ink"
              : "text-muted hover:bg-secondary hover:text-ink",
          ].join(" ")}
          title="Bad response"
          aria-label="Bad response"
          aria-pressed={rating === "down"}
          onClick={() => vote("down")}
        >
          <IconThumbsDown className="h-3.5 w-3.5" />
        </button>
        {onFork && (
          <button
            type="button"
            className="inline-flex h-6 w-6 items-center justify-center rounded-md text-muted transition-colors hover:bg-secondary hover:text-ink disabled:opacity-45"
            title="Fork chat from here"
            aria-label="Fork chat from here"
            disabled={forking}
            onClick={onFork}
          >
            <IconBranch className="h-3.5 w-3.5" />
          </button>
        )}
        <button
          type="button"
          className="inline-flex h-6 w-6 items-center justify-center rounded-md text-muted transition-colors hover:bg-secondary hover:text-ink"
          title="Copy turn"
          aria-label="Copy turn"
          onClick={() => void copyTurn()}
        >
          {copied ? (
            <IconCheck className="h-3.5 w-3.5 text-ok" strokeWidth={2.5} />
          ) : (
            <IconCopy className="h-3.5 w-3.5" />
          )}
        </button>
      </div>
    </div>
  );
}
