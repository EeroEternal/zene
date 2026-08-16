import assert from "node:assert/strict";
import { test } from "node:test";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { capImport, capabilities } from "./capabilities.ts";

const repoRoot = join(dirname(fileURLToPath(import.meta.url)), "../../../..");
const webRoot = join(dirname(fileURLToPath(import.meta.url)), "..");

test("each named capability points at files that exist", () => {
  for (const [id, cap] of Object.entries(capabilities)) {
    assert.ok(cap.symbols.length, `${id} needs symbols`);
    for (const file of cap.files) {
      assert.ok(existsSync(join(repoRoot, file)), `${id}: missing ${file}`);
    }
  }
});

test("each capability has a @/cap/<id> barrel that re-exports its symbols", () => {
  for (const [id, cap] of Object.entries(capabilities)) {
    const barrel = join(webRoot, "lib/cap", `${id}.ts`);
    assert.ok(existsSync(barrel), `missing barrel ${barrel}`);
    const src = readFileSync(barrel, "utf8");
    for (const symbol of cap.symbols) {
      assert.match(src, new RegExp(`\\b${symbol}\\b`), `${id} barrel missing ${symbol}`);
    }
    assert.match(capImport(id), new RegExp(`from "@/cap/${id}"`));
  }
});
