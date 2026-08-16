import assert from "node:assert/strict";
import { test } from "node:test";
import {
  buildConversationTurns,
  turnCopyText,
  turnIndexByEndItemId,
} from "./turnActions.ts";

test("buildConversationTurns joins assistant bubbles in one turn", () => {
  const turns = buildConversationTurns([
    { kind: "bubble", role: "user", text: "go" },
    { kind: "thought", text: "plan" },
    { kind: "bubble", role: "assistant", text: "first" },
    { kind: "tool", text: "ran" },
    { kind: "bubble", role: "assistant", text: "second" },
  ]);
  assert.equal(turns.length, 1);
  assert.equal(turns[0].userText, "go");
  assert.equal(turns[0].assistantText, "first\n\nsecond");
});

test("buildConversationTurns keeps later user messages as new turns", () => {
  const turns = buildConversationTurns([
    { kind: "bubble", role: "user", text: "one" },
    { kind: "bubble", role: "assistant", text: "a" },
    { kind: "bubble", role: "user", text: "two" },
    { kind: "bubble", role: "assistant", text: "b" },
  ]);
  assert.equal(turns.length, 2);
  assert.equal(turns[0].assistantText, "a");
  assert.equal(turns[1].userText, "two");
  assert.equal(turns[1].assistantText, "b");
});

test("turnCopyText copies the whole joined assistant output", () => {
  const [turn] = buildConversationTurns([
    { kind: "bubble", role: "user", text: "q" },
    { kind: "bubble", role: "assistant", text: "part 1" },
    { kind: "bubble", role: "assistant", text: "part 2" },
  ]);
  assert.equal(turnCopyText(turn), "q\n\npart 1\n\npart 2");
});

test("turnIndexByEndItemId points only at the last item of each turn", () => {
  const items = [
    { id: 1, kind: "bubble", role: "user" as const },
    { id: 2, kind: "thought" },
    { id: 3, kind: "bubble", role: "assistant" as const },
    { id: 4, kind: "bubble", role: "assistant" as const },
    { id: 5, kind: "bubble", role: "user" as const },
    { id: 6, kind: "bubble", role: "assistant" as const },
  ];
  const map = turnIndexByEndItemId(items);
  assert.deepEqual([...map.entries()], [
    [4, 0],
    [6, 1],
  ]);
});
