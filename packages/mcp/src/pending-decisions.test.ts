import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import type { Queryable } from "@agentsi/core";

// Core reads the data root when it loads, so AGENTSI_HOME points at the test root before the modules below are imported.
const root = mkdtempSync(join(tmpdir(), "agentsi-mcp-pending-"));
after(() => rmSync(root, { recursive: true, force: true }));
process.env.AGENTSI_HOME = root;
const { pendingDecisionsText } = await import("./pending-decisions.ts");
const core = await import("@agentsi/core");
const { testDb } = await import("@agentsi/core/test-db");

const rowsOf = (rows: unknown[]): Queryable => ({ query: async () => ({ rows }) as never });

test("pending decisions group by minutes and show why and action", async () => {
  const out = await pendingDecisionsText(
    rowsOf([
      { id: "1", minutes_id: "m1", text: "A", why: "because", action: "do it", total: "3" },
      { id: "2", minutes_id: "m1", text: "B", why: null, action: null, total: "3" },
      { id: "3", minutes_id: null, text: "C", why: null, action: null, total: "3" },
    ]),
    "demo",
    10,
  );
  assert.equal(
    out,
    ["minutes m1:", "  #1 A", "     why: because", "     action: do it", "  #2 B", "minutes -:", "  #3 C"].join("\n"),
  );
});

test("pending decisions say when the limit hides some", async () => {
  const out = await pendingDecisionsText(rowsOf([{ id: "1", minutes_id: "m1", text: "A", why: null, action: null, total: "5" }]), "demo", 1);
  assert.match(out, /\(4 more not shown; raise limit\)$/);
});

test("pending decisions with no rows say so", async () => {
  assert.equal(await pendingDecisionsText(rowsOf([]), "demo", 10), "No pending decisions for demo.");
});

test("a missing action column answers Run agentsi migrate, any other error is thrown", async () => {
  const db: Queryable = {
    query: async (text: string) => {
      if (text.includes("schema_migrations")) return { rows: [{ name: "001_init.sql" }] } as never;
      throw Object.assign(new Error('column "action" does not exist'), { code: "42703" });
    },
  };
  assert.match(await pendingDecisionsText(db, "demo", 10), /^Run agentsi migrate\nPending: 002_/);
  const down: Queryable = {
    query: async () => {
      throw new Error("connection refused");
    },
  };
  await assert.rejects(pendingDecisionsText(down, "demo", 10), /connection refused/);
});

// S013 4: restoring an archived decision brings it back everywhere the owner reads it, and agentsi decide accepts it.
// The decisions are a fake table that the readers and `decide` take as a parameter, and the archive is a row of the test database.
// The web list and counts of the same restore are checked in apps/web/lib/decisions.test.ts.

interface Dec {
  id: number;
  text: string;
  why: string | null;
  status: "pending" | "approved" | "rejected";
}

/** The decisions table as the readers and `decide` see it: it answers their queries and keeps a status that `decide` updates. */
function decisionsTable(rows: Dec[]) {
  const full = (r: Dec) => ({
    id: String(r.id),
    product: "agentsi",
    kind: "product",
    minutes_id: null,
    reflection_id: null,
    text: r.text,
    why: r.why,
    action: null,
    status: r.status,
  });
  return {
    decisions: rows,
    query: async (text: string, params: unknown[] = []) => {
      const t = text.replace(/\s+/g, " ");
      if (t.startsWith("update decisions")) {
        const d = rows.find((r) => r.id === Number(params[0]));
        if (d) d.status = params[1] as Dec["status"];
        return { rows: d ? [{ product: "agentsi", minutes_id: null, action: null, pos: "0" }] : [] } as never;
      }
      if (t.includes("where id = $1")) return { rows: rows.filter((r) => r.id === Number(params[0])).map(full) } as never;
      if (t.includes("where product = $1 and status = 'pending'")) {
        const pending = rows.filter((r) => r.status === "pending");
        const shown = t.includes("limit $2") ? pending.slice(0, Number(params[1])) : pending;
        return { rows: shown.map((r) => ({ ...full(r), total: String(pending.length) })) } as never;
      }
      return { rows: [] } as never;
    },
  };
}

/** The product's data root: a registry entry only. The archive record is a row of the test database. */
function initRoot() {
  writeFileSync(
    join(root, "agentsi.config.json"),
    JSON.stringify({ products: [{ id: "agentsi", name: "AgentSI", paths: [root], status: "active", roles: ["pm", "dev"] }] }, null, 2) + "\n",
  );
}

test("restoring an archived decision brings it back to pending_decisions, status and mark --pending, and agentsi decide accepts it", async (t) => {
  initRoot();
  const testDatabase = await testDb();
  t.after(() => testDatabase.close());
  const table = decisionsTable([
    { id: 7, text: "Cut the legacy export", why: "nobody uses it", status: "pending" },
    { id: 8, text: "Ship the pricing page", why: null, status: "pending" },
  ]);
  // The fake table answers the decision queries; the archive record is read from the test database.
  const db = {
    decisions: table.decisions,
    query: (text: string, params: unknown[] = []) => (text.includes("archive_records") ? core.getPool().query(text, params) : table.query(text, params)),
  } as typeof table;
  await core.archiveDecision("agentsi", 7);
  assert.equal(await core.archivedDecisionProduct(7), "agentsi", "decision 7 is archived first, and `agentsi decide` refuses it");
  assert.doesNotMatch(await pendingDecisionsText(db, "agentsi", 10), /#7 /, "pending_decisions leaves it out while it is archived");

  await core.restoreDecision("agentsi", 7);

  assert.match(await pendingDecisionsText(db, "agentsi", 10), /#7 Cut the legacy export/, "pending_decisions returns it again");
  assert.match(await core.statusMarkdown("agentsi", db, root), /\[7\] Cut the legacy export/, "status lists it again");
  const { items } = await core.pendingItems("agentsi", { db, root });
  assert.ok(items.some((i) => i.kind === "decision" && i.id === 7), "mark --pending lists it again");

  assert.equal(await core.archivedDecisionProduct(7), null, "agentsi decide does not refuse it as archived");
  const outcome = await core.decide(7, "approved", { db });
  assert.equal(outcome.status, "approved", "agentsi decide accepts it");
  assert.equal(db.decisions.find((d) => d.id === 7)?.status, "approved");
});
