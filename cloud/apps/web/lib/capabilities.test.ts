import assert from "node:assert/strict";
import { test } from "node:test";
import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { capabilities } from "./capabilities.ts";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "../../../..");

test("each named capability points at files that exist", () => {
  for (const [id, cap] of Object.entries(capabilities)) {
    assert.ok(cap.import.length, `${id} needs an import snippet`);
    for (const file of cap.files) {
      assert.ok(existsSync(join(repoRoot, file)), `${id}: missing ${file}`);
    }
  }
});
