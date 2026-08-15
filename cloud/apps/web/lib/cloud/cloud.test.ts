import assert from "node:assert/strict";
import { test } from "node:test";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const dir = dirname(fileURLToPath(import.meta.url));
const llm = readFileSync(join(dir, "llm.ts"), "utf8");
const repos = readFileSync(join(dir, "repositories.ts"), "utf8");
const runs = readFileSync(join(dir, "runs.ts"), "utf8");
const github = readFileSync(join(dir, "github.ts"), "utf8");
const http = readFileSync(join(dir, "http.ts"), "utf8");

test("typed clients pin the feature slice paths", () => {
  assert.match(llm, /\/api\/v1\/settings\/llm/);
  assert.match(repos, /\/api\/v1\/repositories/);
  assert.match(repos, /\/api\/v1\/repositories\/\$\{repositoryId\}\/branches/);
  assert.match(runs, /\/api\/v1\/runs/);
  assert.match(runs, /\/api\/v1\/runs\/\$\{runId\}\/messages/);
  assert.match(github, /\/api\/v1\/github\/status/);
  assert.match(github, /\/api\/v1\/github\/sync/);
});

test("clients use http helpers instead of raw api()", () => {
  assert.match(http, /export function getJson/);
  assert.match(http, /export function postJson/);
  assert.doesNotMatch(llm, /\bapi</);
  assert.doesNotMatch(repos, /\bapi</);
  assert.doesNotMatch(runs, /\bapi</);
  assert.doesNotMatch(github, /\bapi</);
});
