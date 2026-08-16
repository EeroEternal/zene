import assert from "node:assert/strict";
import { test } from "node:test";
import { applyComposerInsert, detectComposerTrigger, filterSkillsByQuery } from "./composerTriggers.ts";

test("detects slash skill at start", () => {
  const t = detectComposerTrigger("/re", 3);
  assert.deepEqual(t, { kind: "slash", query: "re", start: 0, end: 3 });
});

test("detects slash after space", () => {
  const t = detectComposerTrigger("please /fix", 11);
  assert.deepEqual(t, { kind: "slash", query: "fix", start: 7, end: 11 });
});

test("detects mention token", () => {
  const t = detectComposerTrigger("see @App", 8);
  assert.deepEqual(t, { kind: "mention", query: "App", start: 4, end: 8 });
});

test("ignores completed tokens", () => {
  assert.equal(detectComposerTrigger("/review more", 12), null);
  assert.equal(detectComposerTrigger("hello", 5), null);
});

test("applies insert over the trigger", () => {
  const trigger = detectComposerTrigger("please /re", 10);
  assert.ok(trigger);
  assert.equal(applyComposerInsert("please /re", trigger, "/review "), "please /review ");
});

test("filters skills by label or insert", () => {
  const skills = [
    { label: "Code review", insert: "/review " },
    { label: "Fix bugs", insert: "/fix " },
  ];
  assert.equal(filterSkillsByQuery(skills, "rev").length, 1);
  assert.equal(filterSkillsByQuery(skills, "fix").length, 1);
  assert.equal(filterSkillsByQuery(skills, "").length, 2);
});
