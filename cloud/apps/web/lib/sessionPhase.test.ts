import assert from "node:assert/strict";
import { test } from "node:test";
import { composerChrome, sessionPhase, waitingTurnCopy } from "./sessionPhase.ts";

test("sessionPhase treats a pending follow-up as live so Stop is available", () => {
  assert.equal(sessionPhase("waiting_for_user", false, true), "live");
  assert.equal(sessionPhase("completed", false, true), "live");
  assert.equal(sessionPhase("cancelled", false, true), "live");
  assert.equal(composerChrome(sessionPhase("completed", false, true)).primaryAction, "stop");
});

test("sessionPhase keeps setup and stopping ahead of pending", () => {
  assert.equal(sessionPhase("queued", false, true), "setup");
  assert.equal(sessionPhase("cloning", false, true), "setup");
  assert.equal(sessionPhase("stopping", false, true), "stopping");
  assert.equal(composerChrome(sessionPhase("queued", false, true)).primaryAction, "stop");
});

test("sessionPhase stays idle when nothing is pending", () => {
  assert.equal(sessionPhase("waiting_for_user"), "idle");
  assert.equal(sessionPhase("completed"), "idle");
  assert.equal(composerChrome(sessionPhase("completed")).primaryAction, "send");
});

test("waitingTurnCopy rotates while the first tokens are late", () => {
  assert.match(waitingTurnCopy(800, "running").detail, /Connecting/);
  assert.match(waitingTurnCopy(5000, "running").detail, /Warming context/i);
  assert.match(waitingTurnCopy(10000, "running").detail, /first response tokens/i);
  assert.match(waitingTurnCopy(20000, "running").detail, /Still generating/i);
  assert.equal(waitingTurnCopy(3000, "cloning").title.includes("Cloning") || waitingTurnCopy(3000, "cloning").detail.length > 0, true);
});
