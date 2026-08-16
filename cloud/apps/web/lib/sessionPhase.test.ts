import assert from "node:assert/strict";
import { test } from "node:test";
import { composerChrome, sessionPhase } from "./sessionPhase.ts";

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
