export type ComposerTriggerKind = "slash" | "mention";

export interface ComposerTrigger {
  kind: ComposerTriggerKind;
  query: string;
  start: number;
  end: number;
}

/** Detect `/skill` or `@file` token immediately before the caret. */
export function detectComposerTrigger(text: string, cursor: number): ComposerTrigger | null {
  const pos = Math.max(0, Math.min(cursor, text.length));
  const before = text.slice(0, pos);
  const match = /(?:^|[\s])([/@])(\S*)$/.exec(before);
  if (!match) return null;
  const token = match[1];
  const query = match[2];
  const start = before.length - query.length - 1;
  return {
    kind: token === "/" ? "slash" : "mention",
    query,
    start,
    end: pos,
  };
}

export function applyComposerInsert(text: string, trigger: ComposerTrigger, insert: string): string {
  return text.slice(0, trigger.start) + insert + text.slice(trigger.end);
}

export function filterSkillsByQuery<T extends { label: string; insert: string }>(
  skills: T[],
  query: string,
): T[] {
  const q = query.trim().toLowerCase();
  if (!q) return skills;
  return skills.filter((s) => {
    const insert = s.insert.replace(/^\//, "").toLowerCase();
    return s.label.toLowerCase().includes(q) || insert.startsWith(q) || insert.includes(q);
  });
}
