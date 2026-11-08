import assert from "node:assert/strict";
import { test } from "node:test";
import { reindexCodeGraph } from "./code-graph.ts";

test("a failed reindex logs one line and returns null", async () => {
  const lines: string[] = [];
  assert.equal(await reindexCodeGraph("/", (l) => lines.push(l)), null);
  assert.equal(lines.length, 1);
  assert.match(lines[0]!, /^code graph: no reindex of \/: /);
});
