import assert from "node:assert/strict";
import { test } from "node:test";
import { diffLines, isPast, percent } from "./roles-view";

test("a unified diff is classified line by line", () => {
  const kinds = diffLines("--- a/x\n+++ b/x\n@@ -1 +1 @@\n-old\n+new\n same\n").map((l) => l.kind);
  assert.deepEqual(kinds, ["meta", "meta", "hunk", "del", "add", "ctx"]);
});

test("a due date is past only before today", () => {
  const today = new Date("2026-10-13T12:00:00Z");
  assert.equal(isPast("2026-10-12", today), true);
  assert.equal(isPast("2026-10-13", today), false);
  assert.equal(isPast("soon", today), false);
});

test("percent keeps null as a dash", () => {
  assert.equal(percent(0.666), "67%");
  assert.equal(percent(null), "—");
});
