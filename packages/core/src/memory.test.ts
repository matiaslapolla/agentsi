import assert from "node:assert/strict";
import { existsSync, rmSync } from "node:fs";
import { test } from "node:test";
import * as briefStore from "./brief.ts";
import { getPool } from "./db.ts";
import {
  addFacts,
  appendFacts,
  bumpCounter,
  makeItem,
  readMemoryItems,
  renderForPrompt,
  retireItem,
  type MemoryItem,
} from "./memory.ts";
import { workspaceDir } from "./paths.ts";
import * as rosterStore from "./roster.ts";
import { testDb } from "./test-db.ts";

const at = (day: number) => new Date(Date.UTC(2026, 9, day));
const item = (text: string, day: number, taken: string[] = []): MemoryItem =>
  makeItem(text, { source: "m1", created_at: at(day).toISOString(), taken });

test("item ids are m- plus 4 to 6 hex chars and unique within the file", () => {
  let items: MemoryItem[] = [];
  items = addFacts(items, Array.from({ length: 300 }, (_, i) => `note ${i}`), "m1", at(1));
  assert.equal(new Set(items.map((i) => i.id)).size, 300);
  for (const i of items) assert.match(i.id, /^m-[0-9a-f]{4,6}$/);
});

test("addFacts skips exact duplicates after whitespace normalization and ignores empty notes", () => {
  const first = addFacts([], ["Uses  Postgres 17", "  "], "m1", at(1));
  assert.equal(first.length, 1);
  const second = addFacts(first, [" Uses Postgres\n17 ", "Another fact", "Another fact"], "m2", at(2));
  assert.deepEqual(second.map((i) => i.text), ["Uses Postgres 17", "Another fact"]);
  assert.equal(second[1]!.kind, "fact");
  assert.equal(second[1]!.source, "m2");
});

test("a retired item does not block re-adding its text", () => {
  const first = addFacts([], ["old"], "m1", at(1));
  const retired = retireItem(first, first[0]!.id, "why", at(2));
  assert.equal(addFacts(retired, ["old"], "m2", at(3)).length, 2);
});

test("renderForPrompt lists active items newest first and omits retired ones", () => {
  const items = [item("oldest", 1), item("newest", 3), item("gone", 5), item("middle", 2)].map((i, n) => ({ ...i, id: `m-000${n}` }));
  const text = renderForPrompt(retireItem(items, "m-0002", "x", at(6)), Infinity);
  assert.equal(text, "- [m-0001] newest\n- [m-0003] middle\n- [m-0000] oldest");
});

test("renderForPrompt never cuts an item and counts the ones left out", () => {
  const items = [item("alpha one", 1), item("beta two", 2), item("gamma three", 3)].map((i, n) => ({ ...i, id: `m-000${n}` }));
  const full = renderForPrompt(items, Infinity);
  assert.equal(full, "- [m-0002] gamma three\n- [m-0001] beta two\n- [m-0000] alpha one");
  const tight = renderForPrompt(items, 60);
  assert.equal(tight, "- [m-0002] gamma three\n(2 older items not shown)");
  assert.ok(tight.length <= 60);
  assert.equal(renderForPrompt(items, 10), "(3 older items not shown)");
  assert.equal(renderForPrompt([], 100), "");
});

test("retireItem and bumpCounter are pure and reject unknown ids", () => {
  const items = [{ ...item("a", 1), id: "m-aaaa" }];
  const retired = retireItem(items, "m-aaaa", "stale", at(2));
  assert.equal(retired[0]!.retired_why, "stale");
  assert.equal(items[0]!.retired_at, null);
  assert.equal(bumpCounter(items, "m-aaaa", "harmful")[0]!.harmful, 1);
  assert.throws(() => bumpCounter(items, "m-zzzz", "helpful"));
});

const OVERLAY = "---\nextends: dev\neffort: high\n---\nOnly the API.\n";

const BRIEF = {
  version: 1 as const,
  product: { what: "A demo", audience: "Everyone", stage: "building" as const },
  objectives: [{ id: "o1", text: "Ship", signal: "released", due: "2027-01-01" }],
  north_stars: [],
  sources_of_truth: [],
  out_of_scope: ["mobile"],
  roles: { dev: "judges cost", "dev.backend": "owns the API" },
  updated_at: "2026-10-13",
};

const countOf = async (table: string): Promise<number> =>
  Number(((await getPool().query(`select count(*) as n from ${table}`)).rows[0] as { n: string | number }).n);

test("memory, a roster overlay and a brief round-trip through the database and write no files", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const product = `_test-memory-${process.pid}`;
  t.after(() => rmSync(workspaceDir(product), { recursive: true, force: true }));
  rmSync(workspaceDir(product), { recursive: true, force: true });

  // Memory goes in through appendFacts and comes back through readMemoryItems.
  assert.equal(await appendFacts(product, "dev", ["one", "two"], "min-1", at(1)), 2);
  assert.equal(await appendFacts(product, "dev", ["two", "three"], "min-2", at(2)), 1);
  assert.deepEqual((await readMemoryItems(product, "dev")).map((i) => i.text), ["one", "two", "three"]);
  await appendFacts(product, "dev.backend", ["spec"], "min-3", at(3));
  assert.deepEqual((await readMemoryItems(product, "dev.backend")).map((i) => i.text), ["spec"]);
  assert.deepEqual(await readMemoryItems(product, "qa"), []);
  await assert.rejects(appendFacts(product, "../x", ["n"], "m", at(1)));

  // The overlay and the brief go in through the stores of the roster and the brief.
  const writeOverlay = (rosterStore as Record<string, unknown>).writeOverlay as (p: string, agent: string, text: string) => Promise<void>;
  const writeBrief = (briefStore as Record<string, unknown>).writeBrief as (p: string, brief: briefStore.Brief) => Promise<void>;
  assert.equal(typeof writeOverlay, "function", "roster.ts exports writeOverlay(product, agent, text)");
  assert.equal(typeof writeBrief, "function", "brief.ts exports writeBrief(product, brief)");
  await writeOverlay(product, "dev.backend", OVERLAY);
  await writeBrief(product, BRIEF);

  assert.deepEqual(await briefStore.readBrief(product), BRIEF);
  assert.equal(await briefStore.readBrief(`${product}-none`), null);
  const agent = await rosterStore.loadAgent("dev.backend", product);
  assert.equal(agent.source, "overlay");
  assert.equal(agent.effort, "high");
  assert.match(agent.body, /Only the API\./);
  assert.equal(agent.body.trim().endsWith("Only the API."), true);

  // Each kind sits in its own table.
  assert.equal(await countOf("memory") > 0, true, "memory rows");
  assert.equal(await countOf("agent_overlays"), 1);
  assert.equal(await countOf("briefs"), 1);

  // Nothing was written under the data root.
  assert.equal(existsSync(workspaceDir(product)), false, "no workspace folder for the product");
});
