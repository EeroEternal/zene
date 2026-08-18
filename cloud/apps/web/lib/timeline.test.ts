import assert from "node:assert/strict";
import { test } from "node:test";
import {
  buildTimelineFromEvents,
  groupTimeline,
  timelineHasLiveMeta,
  type TimelineItem,
} from "./timeline.ts";
import type { RunEvent } from "./types.ts";

function thought(partial: Partial<Extract<TimelineItem, { kind: "thought" }>> & { id: number }): TimelineItem {
  return {
    kind: "thought",
    text: partial.text ?? "hmm",
    expanded: false,
    sealed: false,
    startedAt: 1,
    ...partial,
  };
}

test("groupTimeline peels a live thought out of a collapsed activity group", () => {
  const items: TimelineItem[] = [
    thought({ id: 1, text: "old", sealed: true, endedAt: 2 }),
    {
      kind: "tool",
      id: 2,
      toolCallId: "c1",
      title: "Read a.rs",
      toolKind: "read",
      status: "completed",
      expanded: false,
    },
    thought({ id: 3, text: "now working", sealed: false }),
  ];
  const segs = groupTimeline(items);
  assert.equal(segs.length, 2);
  assert.equal(segs[0]?.type, "activity");
  assert.equal(segs[1]?.type, "activity");
  if (segs[0]?.type !== "activity" || segs[1]?.type !== "activity") return;
  assert.equal(segs[0].items.length, 2);
  assert.equal(segs[0].live, false);
  assert.equal(segs[1].items.length, 1);
  assert.equal(segs[1].live, true);
  assert.equal(segs[1].items[0]?.kind, "thought");
});

test("replaying an in-progress thought keeps the trailing thought live", () => {
  const events: RunEvent[] = [
    { seq: 1, createdAt: "2026-08-17T00:00:00Z", eventType: "thought_delta", payload: { text: "The user" } },
    { seq: 2, createdAt: "2026-08-17T00:00:01Z", eventType: "thought_delta", payload: { text: " is asking" } },
  ];
  const draft = buildTimelineFromEvents(events);
  assert.equal(draft.items.length, 1);
  assert.equal(draft.items[0]?.kind, "thought");
  if (draft.items[0]?.kind !== "thought") return;
  assert.equal(draft.items[0].sealed, false);
  assert.equal(draft.items[0].text, "The user is asking");
  assert.equal(timelineHasLiveMeta(draft.items), true);
});
