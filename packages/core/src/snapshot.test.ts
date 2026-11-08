import assert from "node:assert/strict";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { getPool, type Queryable } from "./db.ts";
import { dbSections, storiesSection } from "./snapshot.ts";
import { testDb } from "./test-db.ts";

const fake = (answer: (text: string) => unknown[]): Queryable => ({
  query: async (text: string) => ({ rows: answer(text) }) as never,
});

test("the snapshot lists issues, minutes with marks and pending decisions with their action", async () => {
  const db = fake((text) => {
    if (text.includes("from documents")) return [{ ref: "12", title: "Fix login" }];
    if (text.includes("from minutes")) return [{ id: "m1", question: "What next?", created_at: new Date() }];
    if (text.includes("from priorities")) return [{ idx: 0, title: "Ship it", mark: "accepted", is_new: true }];
    if (text.includes("from decisions")) return [{ id: "7", text: "Adopt X", action: "run Y", minutes_id: "m1" }];
    return [];
  });
  const out = (await dbSections("demo", null, db)).join("\n");
  assert.match(out, /- #12 Fix login/);
  assert.match(out, /### m1 — What next\?/);
  assert.match(out, /#0 .*Ship it \(new\)/);
  assert.match(out, /- \[7\] Adopt X → run Y/);
});

test("the snapshot says so when nothing is indexed", async () => {
  const out = (await dbSections("demo", null, fake(() => []))).join("\n");
  assert.match(out, /_None indexed\._/);
  assert.match(out, /_No meetings yet\._/);
  assert.match(out, /## Pending decisions\n\n_None\._/);
});

test("a database error becomes one section with the message", async () => {
  const db: Queryable = {
    query: async () => {
      throw new Error('column "action" does not exist');
    },
  };
  const out = await dbSections("demo", null, db);
  assert.equal(out.length, 1);
  assert.match(out[0]!, /## Database sections unavailable\n\ncolumn "action" does not exist/);
});

test("the Stories section lists the open stories and counts the closed ones, and is absent without stories", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const root = mkdtempSync(join(tmpdir(), "agentsi-snap-stories-"));
  assert.equal(await storiesSection("p", root), null);
  const write = (id: string, status: string, approval: string, session: object | null) =>
    getPool().query("insert into stories (product, id, data) values ($1, $2, $3::jsonb)", ["p", id, JSON.stringify({ id, title: `Historia ${id}`, status, approval, session })]);
  await write("S001", "not_implemented", "approved", { state: "ready" });
  await write("T002", "implemented", "approved", null);
  await write("S003", "discarded", "pending", null);
  await write("S004", "not_implemented", "pending", null);
  assert.equal(
    await storiesSection("p", root),
    [
      "## Stories",
      "",
      "- S001 Historia S001 — not_implemented, approval approved, session ready",
      "- S004 Historia S004 — not_implemented, approval pending, session none",
      "",
      "Implemented: 1. Discarded: 1.",
    ].join("\n"),
  );
});
