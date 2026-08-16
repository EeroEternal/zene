import type { MessageRole, RunMessage } from "@/lib/types";

export type TurnRating = "up" | "down";

export interface ConversationTurn {
  index: number;
  userText: string;
  assistantText: string;
  /** ISO timestamp of the assistant message when known. */
  assistantAt?: string;
}

const RATING_STORAGE_KEY = "zc.turnRatings";

export function buildConversationTurns(
  items: Array<{ kind: string; role?: MessageRole; text?: string }>,
  messages: RunMessage[] = [],
): ConversationTurn[] {
  const assistantTimes = messages
    .filter((m) => (m.role || "").toLowerCase() === "assistant")
    .map((m) => m.createdAt);

  const turns: ConversationTurn[] = [];
  let userText = "";
  let assistantParts: string[] = [];

  const flush = () => {
    if (!userText) return;
    const idx = turns.length;
    turns.push({
      index: idx,
      userText,
      assistantText: assistantParts.join("\n\n"),
      assistantAt: assistantTimes[idx],
    });
    userText = "";
    assistantParts = [];
  };

  for (const item of items) {
    if (item.kind !== "bubble") continue;
    if (item.role === "user") {
      flush();
      userText = item.text || "";
    } else if (item.role === "assistant" && userText && item.text) {
      assistantParts.push(item.text);
    }
  }
  flush();
  return turns;
}

/** Maps the last timeline item of each turn (not the user bubble) to that turn index. */
export function turnIndexByEndItemId(
  items: Array<{ id: number; kind: string; role?: MessageRole }>,
): Map<number, number> {
  const lastId: number[] = [];
  let turnIdx = -1;
  for (const item of items) {
    if (item.kind === "bubble" && item.role === "user") {
      turnIdx += 1;
      continue;
    }
    if (turnIdx >= 0) lastId[turnIdx] = item.id;
  }
  const byItem = new Map<number, number>();
  for (let i = 0; i < lastId.length; i++) {
    if (lastId[i] != null) byItem.set(lastId[i], i);
  }
  return byItem;
}

export function turnCopyText(turn: ConversationTurn): string {
  if (!turn.assistantText.trim()) return turn.userText;
  return `${turn.userText}\n\n${turn.assistantText}`;
}

export function buildForkPrompt(turns: ConversationTurn[], throughIndex: number): string {
  const lines: string[] = [];
  for (let i = 0; i <= throughIndex && i < turns.length; i++) {
    const t = turns[i];
    lines.push(`User:\n${t.userText}`);
    if (t.assistantText.trim()) {
      lines.push(`Assistant:\n${t.assistantText}`);
    }
  }
  lines.push("", "---", "Continue this conversation from here.");
  return lines.join("\n");
}

export function formatRelativeTime(iso?: string): string | null {
  if (!iso) return null;
  const then = new Date(iso).getTime();
  if (!Number.isFinite(then)) return null;
  const secs = Math.max(0, Math.round((Date.now() - then) / 1000));
  if (secs < 60) return `${secs}s ago`;
  const mins = Math.floor(secs / 60);
  if (mins < 60) return `${mins}m ago`;
  const hours = Math.floor(mins / 60);
  if (hours < 48) return `${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `${days}d ago`;
}

function readRatings(): Record<string, TurnRating> {
  try {
    const raw = localStorage.getItem(RATING_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as Record<string, TurnRating>;
    return parsed && typeof parsed === "object" ? parsed : {};
  } catch {
    return {};
  }
}

function writeRatings(map: Record<string, TurnRating>) {
  try {
    localStorage.setItem(RATING_STORAGE_KEY, JSON.stringify(map));
  } catch {
    /* ignore */
  }
}

export function ratingKey(runId: string, turnIndex: number): string {
  return `${runId}:${turnIndex}`;
}

export function loadTurnRating(runId: string, turnIndex: number): TurnRating | null {
  const map = readRatings();
  return map[ratingKey(runId, turnIndex)] ?? null;
}

export function saveTurnRating(
  runId: string,
  turnIndex: number,
  rating: TurnRating,
): TurnRating | null {
  const key = ratingKey(runId, turnIndex);
  const map = readRatings();
  const prev = map[key] ?? null;
  if (prev === rating) {
    delete map[key];
    writeRatings(map);
    return null;
  }
  map[key] = rating;
  writeRatings(map);
  return rating;
}
