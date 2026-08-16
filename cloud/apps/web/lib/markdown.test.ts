import assert from "node:assert/strict";
import { test } from "node:test";
import { balanceMarkdownFences, normalizeMarkdown } from "./markdown.ts";

test("normalizeMarkdown does not split bold labels at a Chinese colon", () => {
  const src = "**关键发现 1：SGLang 对标从未跑过**\n\n**关键发现 2：GPU 被占**";
  assert.equal(normalizeMarkdown(src), src);
});

test("normalizeMarkdown still splits a glued English status opener", () => {
  assert.equal(normalizeMarkdown("好的。Now I'll search the repo"), "好的。\n\nNow I'll search the repo");
});

test("balanceMarkdownFences closes a dangling fence at the end", () => {
  const src = "intro\n```\ncode line\n";
  assert.equal(balanceMarkdownFences(src), "intro\n```\ncode line\n\n```");
});

test("balanceMarkdownFences closes a fence before a swallowed heading", () => {
  const src = "```\nGPU 0 → 87GB\n\nV4 TP=8 需要全部 8 张卡。\n## 回答你的原始问题（更好）\n后面还有正文";
  const out = balanceMarkdownFences(src);
  assert.match(out, /```\n## 回答你的原始问题/);
  assert.ok(out.includes("后面还有正文"));
});
