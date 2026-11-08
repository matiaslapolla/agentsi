import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";
import { pendingItems } from "./pending.ts";
import { getPool, type TxClient } from "./db.ts";
import { testDb } from "./test-db.ts";
import { combineCommits, persistMarks, setMarks, type MarkEntry } from "./minutes.ts";
import type { DecisionStore } from "./reflection-decision.ts";
import type { Minutes, Priority } from "./types.ts";

const prio = (title: string, mark: Priority["mark"] = null): Priority => ({ title, why: "w", sources: [], owner_role: "pm", mark, is_new: false });
const minutes = (id: string, priorities: Priority[], status: Minutes["status"] = "valid"): Minutes => ({
  id, product: "demo", scope: null, question: "q", created_at: "2026-10-17T00:00:00Z", roles: {}, priorities,
  disagreements: [], decisions: [], script: [], check: { passed: true, failures: [], repaired: false }, status,
});

/** Puts the minutes in a test database, closed when the test ends. The decisions come from the fake store of the test. */
async function seed(t: TestContext, list: Minutes[]): Promise<string> {
  const db = await testDb();
  t.after(() => db.close());
  for (const m of list) await getPool().query("insert into minutes (id, product, data) values ($1, 'demo', $2::jsonb)", [m.id, JSON.stringify(m)]);
  return mkdtempSync(join(tmpdir(), "pending-"));
}

const store = (rows: object[]): DecisionStore => ({ async query<R extends object>() { return { rows: rows as R[] }; } });

test("pending items list unmarked priorities and decisions, oldest minutes first, product decisions last", async (t) => {
  const root = await seed(t, [
    minutes("2026-10-17-1000-b", [prio("B0", "accepted"), prio("B1")]),
    minutes("2026-10-16-1000-a", [prio("A0"), prio("A1", "rejected")]),
    minutes("2026-10-15-1000-bad", [prio("X")], "invalid"),
  ]);
  const db = store([
    { id: "3", minutes_id: null, text: "product", why: null, action: null },
    { id: "2", minutes_id: "2026-10-17-1000-b", text: "d-b", why: "y", action: "run it" },
    { id: "1", minutes_id: "2026-10-16-1000-a", text: "d-a", why: null, action: null },
  ]);
  const { items, decisions_error } = await pendingItems("demo", { root, db });
  assert.equal(decisions_error, undefined);
  assert.deepEqual(
    items.map((i) => (i.kind === "priority" ? `${i.minutes_id}#${i.idx}` : `d${i.id}`)),
    ["2026-10-16-1000-a#0", "d1", "2026-10-17-1000-b#1", "d2", "d3"],
  );
});

test("pending items keep the priorities and report the error when the decisions cannot be read", async (t) => {
  const root = await seed(t, [minutes("m1", [prio("A")])]);
  const down: DecisionStore = { async query() { throw new Error("connection refused"); } };
  const res = await pendingItems("demo", { root, db: down });
  assert.equal(res.items.length, 1);
  assert.equal(res.decisions_error, "connection refused");
});

test("pending items are empty when nothing waits", async (t) => {
  const root = await seed(t, [minutes("m1", [prio("A", "accepted")])]);
  assert.deepEqual(await pendingItems("demo", { root, db: store([]) }), { items: [] });
});

test("setMarks changes nothing when one entry is invalid", () => {
  const byId = new Map([["m1", minutes("m1", [prio("A"), prio("B")])]]);
  assert.throws(
    () => setMarks(byId, [{ minutesId: "m1", idx: 0, mark: "accepted", isNew: false }, { minutesId: "m1", idx: 5, mark: "rejected", isNew: false }]),
    /no priority #5/,
  );
  assert.equal(byId.get("m1")!.priorities[0]!.mark, null);
  assert.throws(() => setMarks(byId, [{ minutesId: "zz", idx: 0, mark: "accepted", isNew: false }]), /not found/);
  setMarks(byId, [{ minutesId: "m1", idx: 1, mark: "changed", isNew: true }]);
  assert.deepEqual([byId.get("m1")!.priorities[1]!.mark, byId.get("m1")!.priorities[1]!.is_new], ["changed", true]);
});

test("setMarks rejects a priority twice in one batch and, with unmarkedOnly, another mark on a marked priority", () => {
  const byId = new Map([["m1", minutes("m1", [prio("A"), prio("B", "accepted")])]]);
  assert.throws(
    () => setMarks(byId, [{ minutesId: "m1", idx: 0, mark: "accepted", isNew: false }, { minutesId: "m1", idx: 0, mark: "rejected", isNew: false }]),
    /appears twice/,
  );
  assert.equal(byId.get("m1")!.priorities[0]!.mark, null);
  assert.throws(() => setMarks(byId, [{ minutesId: "m1", idx: 1, mark: "rejected", isNew: false }], { unmarkedOnly: true }), /already marked accepted/);
  setMarks(byId, [{ minutesId: "m1", idx: 1, mark: "accepted", isNew: false }], { unmarkedOnly: true });
  setMarks(byId, [{ minutesId: "m1", idx: 1, mark: "rejected", isNew: false }]);
  assert.equal(byId.get("m1")!.priorities[1]!.mark, "rejected");
});

/** A fake client that logs the first word of each statement. `fail` throws on statements that start with it. */
function tx(o: { fail?: string[]; updateRows?: number; rowExists?: boolean } = {}) {
  const sql: string[] = [];
  let released = false;
  const client: TxClient = {
    async query<R extends object>(text: string) {
      const verb = text.split(" ")[0]!;
      sql.push(verb);
      if (o.fail?.includes(verb)) throw new Error(`${verb} failed`);
      if (verb === "update") return { rows: Array.from({ length: o.updateRows ?? 1 }, () => ({})) as R[] };
      if (verb === "select") return { rows: (o.rowExists ? [{}] : []) as R[] };
      return { rows: [] as R[] };
    },
    release() { released = true; },
  };
  return { client, sql, released: () => released };
}

const entry = (minutesId: string): MarkEntry => ({ minutesId, idx: 0, mark: "accepted", isNew: false });

function batch() {
  const before = new Map([["m1", minutes("m1", [prio("A")])], ["m2", minutes("m2", [prio("B")])]]);
  const after = structuredClone(before);
  for (const m of after.values()) m.priorities[0]!.mark = "accepted";
  return { entries: [entry("m1"), entry("m2")], before, after };
}

test("persistMarks writes no file and rolls back when a row update fails", async () => {
  const written: string[] = [];
  const t = tx({ fail: ["update"] });
  await assert.rejects(persistMarks(batch(), t.client, {}, (m) => written.push(m.id)), /update failed/);
  assert.deepEqual(written, []);
  assert.deepEqual(t.sql, ["begin", "update", "rollback"]);
  assert.ok(t.released());
});

test("persistMarks puts back the first file when the second write fails", async () => {
  const files = new Map<string, string | null>();
  const write = (m: Minutes) => {
    if (m.id === "m2" && m.priorities[0]!.mark) throw new Error("disk full");
    files.set(m.id, m.priorities[0]!.mark);
  };
  const t = tx();
  await assert.rejects(persistMarks(batch(), t.client, {}, write), /disk full/);
  assert.deepEqual(Object.fromEntries(files), { m1: null, m2: null });
  assert.deepEqual(t.sql, ["begin", "update", "update", "rollback"]);
});

test("persistMarks puts back the files when the commit fails", async () => {
  const files = new Map<string, string | null>();
  await assert.rejects(persistMarks(batch(), tx({ fail: ["commit"] }).client, {}, (m) => files.set(m.id, m.priorities[0]!.mark)), /commit failed/);
  assert.deepEqual(Object.fromEntries(files), { m1: null, m2: null });
});

test("persistMarks keeps the original error when the rollback fails too", async () => {
  await assert.rejects(persistMarks(batch(), tx({ fail: ["update", "rollback"] }).client, {}, () => {}), /update failed/);
});

test("persistMarks with unmarkedOnly refuses a row that another mark reached first", async () => {
  const written: string[] = [];
  await assert.rejects(persistMarks(batch(), tx({ updateRows: 0, rowExists: true }).client, { unmarkedOnly: true }, (m) => written.push(m.id)), /already marked/);
  assert.equal(written.length, 0);
  await persistMarks(batch(), tx({ updateRows: 0, rowExists: false }).client, { unmarkedOnly: true }, (m) => written.push(m.id));
  assert.deepEqual(written, ["m1", "m2"]);
});

test("combineCommits keeps the first error whatever the order", () => {
  const fail = { committed: false, error: "A failed" };
  assert.deepEqual(combineCommits([fail, { committed: true }]), fail);
  assert.deepEqual(combineCommits([{ committed: true }, fail]), fail);
  assert.deepEqual(combineCommits([{ committed: false }, { committed: true }]), { committed: true });
  assert.deepEqual(combineCommits([]), { committed: false });
});

test("a decision of minutes written before the links lists no linked priorities, and approving it marks no priority", async (t) => {
  const old = { ...minutes("2026-10-16-1000-old", [prio("A"), prio("B")]), decisions: [{ text: "Cortar X", why: "cuesta", action: "Borrar X" }] };
  const root = await seed(t, [old]);
  // The decisions table as the database holds it. pendingItems reads only the rows still pending, as its SQL asks.
  const table = [{ id: "4", minutes_id: old.id, text: "Cortar X", why: "cuesta", action: "Borrar X", status: "pending" }];
  const db: DecisionStore = {
    async query<R extends object>(sql: string) {
      return { rows: table.filter((r) => sql.includes("status = 'pending'") && r.status === "pending") as unknown as R[] };
    },
  };

  const before = (await pendingItems("demo", { root, db })).items;
  const decision = before.find((i) => i.kind === "decision") as Record<string, unknown> | undefined;
  assert.ok(decision, "the decision waits for the owner");
  assert.deepEqual(decision.priorities, []);
  assert.deepEqual(
    before.map((i) => (i.kind === "priority" ? `${i.minutes_id}#${i.idx}` : `d${i.id}`)),
    ["2026-10-16-1000-old#0", "2026-10-16-1000-old#1", "d4"],
  );

  // The owner approves it: the database row leaves pending. Only the decision changes.
  table[0]!.status = "approved";
  const after = (await pendingItems("demo", { root, db })).items;
  assert.deepEqual(
    after.map((i) => (i.kind === "priority" ? `${i.minutes_id}#${i.idx}` : `d${i.id}`)),
    ["2026-10-16-1000-old#0", "2026-10-16-1000-old#1"],
  );
});
