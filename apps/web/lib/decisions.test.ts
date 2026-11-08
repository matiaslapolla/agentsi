import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, before, describe, test } from "node:test";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { decisionLink, decisionStatus, withRows } from "./decisions";

test("a pending decision links to its minutes, a reflection to the Roles page, a product decision nowhere", () => {
  assert.deepEqual(decisionLink("demo", { kind: "minutes", minutes_id: "2026-10-13-1500-q", reflection_id: null }), {
    href: "/p/demo/m/2026-10-13-1500-q",
    label: "2026-10-13-1500-q",
  });
  assert.deepEqual(decisionLink("demo", { kind: "reflection", minutes_id: null, reflection_id: "r-2026-10-13-1530" }), {
    href: "/p/demo/roles",
    label: "r-2026-10-13-1530",
  });
  assert.equal(decisionLink("demo", { kind: "product", minutes_id: null, reflection_id: null }), null);
});

test("withRows lists the file decisions even when the DB has more rows", () => {
  const file = [{ text: "a" }, { text: "b" }];
  const rows = [
    { text: "a", status: "approved" as const },
    { text: "b", status: "pending" as const },
    { text: "c", status: "pending" as const },
  ];
  const merged = withRows(file, rows);
  assert.equal(merged.length, 2);
  assert.deepEqual(merged.map(decisionStatus), ["approved", "pending"]);
});

test("decisionStatus falls back to the file status, then to pending", () => {
  assert.equal(decisionStatus({ status: "rejected" }), "rejected");
  assert.equal(decisionStatus({ status: "approved", row: { status: "pending" } }), "pending");
  assert.equal(decisionStatus({}), "pending");
});

// S013 2: an archived decision is absent from the web decisions list and from the product card count by default,
// and both come back when the owner asks for the archived view. The web loaders read the data root when they load
// and the decisions table when they query, so the data root is set first and the table is a fake: the
// `__agentsiPool` seam of lib/db.ts answers the queries the web sends.
const MINUTES = "2026-10-21-0900-roadmap";
const ARCHIVED = 7;
const OPEN = 8;

interface DecisionRow {
  id: string;
  product: string;
  kind: string;
  minutes_id: string | null;
  reflection_id: null;
  text: string;
  why: string | null;
  action: null;
  status: "pending";
}

const decisions: DecisionRow[] = [
  { id: String(ARCHIVED), product: "agentsi", kind: "minutes", minutes_id: MINUTES, reflection_id: null, text: "Cut the legacy export", why: "nobody uses it", action: null, status: "pending" },
  { id: String(OPEN), product: "agentsi", kind: "minutes", minutes_id: MINUTES, reflection_id: null, text: "Ship the pricing page", why: null, action: null, status: "pending" },
];

/** The decisions table as the web reads it: pending rows of one product, and every row for the queries that load the card. */
function decisionsTable() {
  return {
    query: async (text: string, params: unknown[] = []) => {
      const t = text.replace(/\s+/g, " ");
      // The owner archived the decision in the terminal; the web app reads the same record, a row of the database.
      if (/from archive_records/.test(t)) return { rows: params[1] === "decision" ? [{ ref: String(ARCHIVED) }] : [] };
      if (/from minutes where product/.test(t)) {
        return { rows: [{ data: { id: MINUTES, product: "agentsi", question: "What do we build next?", status: "valid", created_at: "2026-10-21T09:00:00Z", roles: {}, priorities: [], decisions: [] } }] };
      }
      if (!/from decisions/.test(t)) return { rows: [] };
      if (/where product = \$1 and status = 'pending'/.test(t)) {
        return { rows: decisions.filter((r) => r.status === "pending" && r.product === params[0]) };
      }
      return { rows: decisions };
    },
  };
}

describe("S013: archived decisions on the web", () => {
  const g = globalThis as { __agentsiPool?: unknown };
  let root = "";

  before(() => {
    root = mkdtempSync(join(tmpdir(), "agentsi-web-decisions-"));
    process.env.AGENTSI_HOME = root;
    process.env.AGENTSI_DATABASE_URL = "postgres://x:x@127.0.0.1:1/x";
    writeFileSync(
      join(root, "agentsi.config.json"),
      JSON.stringify({ products: [{ id: "agentsi", name: "AgentSI", paths: [root], status: "active", roles: ["pm", "dev"] }] }, null, 2) + "\n",
    );
    g.__agentsiPool = decisionsTable();
  });

  after(() => {
    delete g.__agentsiPool;
    rmSync(root, { recursive: true, force: true });
  });

  test("the web decisions list leaves an archived decision out by default", async () => {
    const { pendingDecisions } = await import("./workspace");
    const ids = (await pendingDecisions("agentsi")).map((d) => Number(d.id));
    assert.deepEqual(ids, [OPEN]);
  });

  test("the web decisions list shows an archived decision when the owner asks for archived ones", async () => {
    const { pendingDecisions } = await import("./workspace");
    const ids = (await pendingDecisions("agentsi", { includeArchived: true })).map((d) => Number(d.id));
    assert.deepEqual(ids.sort(), [ARCHIVED, OPEN]);
  });

  /** The product card as the home page draws it, with the quest the inbox computes for the product. */
  async function card(includeArchived: boolean) {
    const { loadInbox } = await import("./inbox-data");
    const { ProductCard } = await import("../components/product-card");
    const state = await (includeArchived ? loadInbox({ includeArchived: true }) : loadInbox());
    const product = { id: "agentsi", name: "AgentSI", status: "active" };
    const quest = state.products.find((p) => p.id === product.id);
    assert.ok(quest, "the product has a card");
    return { quest, html: renderToStaticMarkup(createElement(ProductCard, { product, quest, note: null })) };
  }

  test("the product card counts leave an archived decision out by default", async () => {
    const { quest, html } = await card(false);
    assert.equal(quest.pendingDecisions, 1, "the open decision, not two");
    assert.match(html, /title="1 decisiones pendientes"/, "the card shows the count without the archived decision");
  });

  test("the product card counts an archived decision when the owner asks for archived ones", async () => {
    const { quest, html } = await card(true);
    assert.equal(quest.pendingDecisions, 2, "both decisions");
    assert.match(html, /title="2 decisiones pendientes"/, "the card shows the count with the archived decision");
  });
});
