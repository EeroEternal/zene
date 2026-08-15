import assert from "node:assert/strict";
import { test } from "node:test";
import { filterLabelText, filterRuns, repoLabel } from "./listPrefs.ts";
import type { Repo, Run } from "./types.ts";

const repos: Repo[] = [
  { id: "a", owner: "acme", name: "web" },
  { id: "b", owner: "acme", name: "api" },
];

function run(partial: Partial<Run> & Pick<Run, "id" | "repositoryId">): Run {
  return {
    title: partial.title || partial.id,
    status: partial.status || "completed",
    updatedAt: partial.updatedAt,
    createdAt: partial.createdAt || "2026-01-01T00:00:00Z",
    ...partial,
  } as Run;
}

test("repoLabel formats owner/name", () => {
  assert.equal(repoLabel(repos, "a"), "acme/web");
  assert.equal(repoLabel(repos, "missing"), "missing");
});

test("filterRuns keeps project matches", () => {
  const runs = [run({ id: "1", repositoryId: "a" }), run({ id: "2", repositoryId: "b" })];
  assert.deepEqual(
    filterRuns(runs, "project", "b", "a").map((r) => r.id),
    ["2"],
  );
});

test("filterLabelText describes current project", () => {
  assert.equal(filterLabelText("project", "", repos, "a"), "acme/web");
  assert.equal(filterLabelText("failed", "", repos, "a"), "Failed");
});
