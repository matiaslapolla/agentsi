import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, test } from "node:test";
import type { Queryable } from "@agentsi/core";

// The web modules read the data root when they load, so the root is set before any of them is imported.
const root = mkdtempSync(join(tmpdir(), "agentsi-web-archive-list-"));
process.env.AGENTSI_HOME = root;
process.env.AGENTSI_DATABASE_URL = "postgres://x:x@127.0.0.1:1/x";

const ACTIVE = "2026-10-21-0900-roadmap";
const ARCHIVED = "2026-10-14-0900-old-roadmap";

function meetingFile(id: string, created: string) {
  return {
    id,
    product: "agentsi",
    question: `Question of ${id}`,
    status: "valid",
    created_at: created,
    roles: {},
    priorities: [
      { title: "Ship A", why: "w", owner_role: "pm", objective: null, sources: [], mark: null, is_new: false },
      { title: "Ship B", why: "w", owner_role: "dev", objective: null, sources: [], mark: null, is_new: false },
    ],
    decisions: [{ text: "Cut X", why: null, action: null, status: "pending" }],
  };
}

/** A database with the minutes rows, as the table holds them: the archive record is in the data root, so the table keeps every minutes. */
function minutesDb(): Queryable {
  const rows = [
    { id: ACTIVE, question: `Question of ${ACTIVE}`, status: "valid", created_at: new Date("2026-10-21T09:00:00Z"), marks: {} },
    { id: ARCHIVED, question: `Question of ${ARCHIVED}`, status: "valid", created_at: new Date("2026-10-14T09:00:00Z"), marks: {} },
  ];
  return {
    query: async (text: string, params?: unknown[]) => {
      if (/from archive_records/.test(text)) return (await import("@agentsi/core")).getPool().query(text, params) as never;
      if (/count\(\*\) as n from minutes/.test(text)) return { rows: [{ n: String(rows.length) }] } as never;
      if (/from minutes/.test(text)) return { rows } as never;
      return { rows: [] } as never;
    },
  };
}

const lineOf = (text: string, id: string) => text.split("\n").find((l) => l.includes(id)) ?? "";

let testDatabase: { close(): Promise<void> } | undefined;

before(async () => {
  mkdirSync(root, { recursive: true });
  writeFileSync(
    join(root, "agentsi.config.json"),
    JSON.stringify({ products: [{ id: "agentsi", name: "AgentSI", paths: [root], status: "active", roles: ["pm", "dev"] }] }, null, 2) + "\n",
  );
  // The two meetings are stored in the test database, which the web app reads through the shared pool.
  const { testDb } = await import("@agentsi/core/test-db");
  const { getPool, archiveMinutes } = await import("@agentsi/core");
  testDatabase = await testDb();
  for (const [id, created] of [[ACTIVE, "2026-10-21T09:00:00Z"], [ARCHIVED, "2026-10-14T09:00:00Z"]]) {
    const m = meetingFile(id!, created!);
    await getPool().query("insert into minutes (id, product, question, status, created_at, data) values ($1, 'agentsi', $2, 'valid', $3, $4::jsonb)", [id, m.question, created, JSON.stringify(m)]);
    for (const [i, p] of m.priorities.entries()) {
      await getPool().query("insert into priorities (minutes_id, idx, title, why, owner_role) values ($1, $2, $3, $4, $5)", [id, i, p.title, p.why, p.owner_role]);
    }
    await getPool().query("insert into decisions (product, minutes_id, text, status) values ('agentsi', $1, 'Cut X', 'pending')", [id]);
  }
  // The owner archives the old meeting in the terminal, so the web app and the status see the record the CLI wrote.
  await archiveMinutes("agentsi", ARCHIVED);
});

after(async () => {
  await testDatabase?.close();
  rmSync(root, { recursive: true, force: true });
});

test("the web minutes list hides an archived minutes file by default", async () => {
  const { listMinutes } = await import("./workspace");
  const ids = (await listMinutes("agentsi")).map((m) => m.id);
  assert.deepEqual(ids, [ACTIVE]);
});

test("the web minutes list shows an archived minutes file marked as archived when the owner asks for archived ones", async () => {
  const { listMinutes } = await import("./workspace");
  const all = await listMinutes("agentsi", { includeArchived: true });
  assert.deepEqual(all.map((m) => m.id).sort(), [ACTIVE, ARCHIVED].sort());
  assert.equal(all.find((m) => m.id === ARCHIVED)?.archived, true);
  assert.notEqual(all.find((m) => m.id === ACTIVE)?.archived, true);
});

test("the product card counts leave out an archived minutes file", async () => {
  const { loadInbox } = await import("./inbox-data");
  const state = await loadInbox();
  const card = state.products.find((p) => p.id === "agentsi");
  assert.ok(card, "the product has a card");
  assert.equal(card.unmarked, 2, "the two priorities of the active meeting, not four");
  assert.equal(card.pendingDecisions, 1, "the decision of the active meeting, not two");
  assert.deepEqual(state.minutes.map((m) => m.id), [ACTIVE]);
});

test("agentsi status counts and lists the last minutes without the archived one", async () => {
  const { statusMarkdown } = await import("@agentsi/core");
  const out = await statusMarkdown("agentsi", minutesDb());
  assert.match(out, /- Minutes: 1 total/);
  assert.ok(out.includes(ACTIVE), "the active meeting is listed");
  assert.equal(out.includes(ARCHIVED), false, "the archived meeting is not listed");
});

test("the MCP list_minutes tool leaves out an archived minutes file, and marks it when the owner asks for archived ones", async () => {
  const { listMinutesText } = await import("../../../packages/mcp/src/list-minutes.ts");
  const text = await listMinutesText(minutesDb(), "agentsi", 5);
  assert.ok(text.includes(ACTIVE), "the active meeting is listed");
  assert.equal(text.includes(ARCHIVED), false, "the archived meeting is not listed");

  const shown = await listMinutesText(minutesDb(), "agentsi", 5, { includeArchived: true });
  assert.match(lineOf(shown, ARCHIVED), /archived/, "the archived meeting is marked as archived");
  assert.doesNotMatch(lineOf(shown, ACTIVE), /archived/, "the active meeting is not marked");
});
