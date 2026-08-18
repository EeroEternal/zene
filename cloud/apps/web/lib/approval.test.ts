import assert from "node:assert/strict";
import { test } from "node:test";
import {
  isAskUserApproval,
  matchAskUserApproval,
  parseAskUser,
} from "./approval.ts";
import type { Approval } from "./types.ts";

test("parseAskUser reads question options and generated ids", () => {
  const prompt = parseAskUser({
    askUser: true,
    question: "Create the pull request now?",
    options: [
      { label: "Yes, open a draft PR", description: "Push and open GitHub" },
      { label: "Not yet" },
    ],
  });
  assert.ok(prompt);
  assert.equal(prompt?.question, "Create the pull request now?");
  assert.equal(prompt?.options.length, 2);
  assert.equal(prompt?.options[0].id, "ask-0");
  assert.equal(prompt?.options[0].label, "Yes, open a draft PR");
});

test("parseAskUser reads a timeline tool input JSON string", () => {
  const prompt = parseAskUser(
    JSON.stringify({
      question: "你希望我把什么提交成 PR？",
      options: [{ label: "有新的代码改动要提" }, { label: "先看看有没有其他分支/远端" }],
    }),
  );
  assert.equal(prompt?.question, "你希望我把什么提交成 PR？");
  assert.equal(prompt?.options.length, 2);
  assert.equal(prompt?.options[0].id, "ask-0");
});

test("parseAskUser unwraps approval payload rawInput", () => {
  const prompt = parseAskUser({
    requestId: "ask_1",
    title: "Create the pull request now?",
    rawInput: {
      askUser: true,
      question: "Create the pull request now?",
      options: [{ label: "Yes" }],
    },
  });
  assert.equal(prompt?.question, "Create the pull request now?");
  assert.equal(prompt?.options[0].id, "ask-0");
});

test("matchAskUserApproval pairs by question and skips used ids", () => {
  const first: Approval = {
    id: "a1",
    payload: {
      requestId: "ask_1",
      rawInput: { askUser: true, question: "Ship it?", options: [{ label: "Yes" }] },
    },
  };
  const second: Approval = {
    id: "a2",
    payload: {
      requestId: "ask_2",
      rawInput: { askUser: true, question: "Which branch?", options: [{ label: "main" }] },
    },
  };
  assert.equal(isAskUserApproval(first), true);
  const hit = matchAskUserApproval({ question: "Which branch?" }, [first, second]);
  assert.equal(hit?.id, "a2");
  const used = new Set(["a2"]);
  const fallback = matchAskUserApproval({ question: "Which branch?" }, [first, second], used);
  assert.equal(fallback?.id, "a1");
});
