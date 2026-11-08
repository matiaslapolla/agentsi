import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";
import { join, relative } from "node:path";
import { after, test } from "node:test";
import { testHome } from "./test-home.ts";

const h = testHome([{ id: "agentsi", name: "AgentSI", roles: ["pm", "dev"] }]);
after(h.cleanup);

// Core reads the data root when it loads, so AGENTSI_HOME points at the test home before core is imported.
process.env.AGENTSI_HOME = h.home;
const core = await import("@agentsi/core");
const { describe: describeItem } = await import("./commands/mark-wizard.ts");
const { archive } = await import("./commands/archive.ts");
const { decide } = await import("./commands/decide.ts");
const { runCommand } = await import("./run-command.ts");
const { testDb } = await import("@agentsi/core/test-db");

/** The commands run in this process, against the test database that `getPool()` points at. */
const cli = (name: "archive" | "decide", ...args: string[]) => runCommand(name === "archive" ? archive : decide, args);

type Status = "pending" | "approved" | "rejected";
interface Dec {
  id: number;
  text: string;
  why: string | null;
  status: Status;
}

/**
 * The decisions table of the product, as the readers see it. It answers the queries that status and pending
 * items send, and nothing else. The archive never writes to it, so an archived decision keeps its row.
 */
function decisionsTable(rows: Dec[]) {
  return {
    query: async (text: string) => {
      const t = text.replace(/\s+/g, " ");
      if (!t.includes("from decisions where product = $1 and status = 'pending'")) return { rows: [] } as never;
      const pending = rows.filter((r) => r.status === "pending");
      return {
        rows: pending.map((r) => ({
          id: String(r.id),
          minutes_id: null,
          text: r.text,
          why: r.why,
          action: null,
          total: String(pending.length),
        })),
      } as never;
    },
  };
}

/** Text of `agentsi mark --pending`: the same lines the command prints for the same items. */
function markPendingText(items: Awaited<ReturnType<typeof core.pendingItems>>["items"]): string {
  if (!items.length) return "Nothing pending.";
  return items.map((item, n) => `${n + 1}. ${describeItem(item).join("\n")}`).join("\n");
}

/** The text of every file under `dir`, keyed by its path relative to `dir`. Git metadata is left out. */
function snapshot(dir: string): Map<string, string> {
  const files = new Map<string, string>();
  const walk = (at: string) => {
    for (const entry of readdirSync(at, { withFileTypes: true })) {
      if (entry.name === ".git") continue;
      const path = join(at, entry.name);
      if (entry.isDirectory()) walk(path);
      else files.set(relative(dir, path), readFileSync(path, "latin1"));
    }
  };
  walk(dir);
  return files;
}

/** The paths whose text differs between two snapshots, added and removed ones included. */
function differing(before: Map<string, string>, after: Map<string, string>): string[] {
  return [...new Set([...before.keys(), ...after.keys()])].filter((p) => before.get(p) !== after.get(p));
}

test("archiving a pending decision keeps it pending and takes it off agentsi status and mark --pending", async (t) => {
  const testDatabase = await testDb();
  t.after(() => testDatabase.close());
  const db = decisionsTable([
    { id: 7, text: "Cut the legacy export", why: "nobody uses it", status: "pending" },
    { id: 8, text: "Ship the pricing page", why: null, status: "pending" },
  ]);
  const statusBefore = await core.statusMarkdown("agentsi", db, h.home);
  const pendingBefore = await core.pendingItems("agentsi", { db, root: h.home });
  assert.match(statusBefore, /\[7\] Cut the legacy export/, "status lists decision 7 before the archive");
  assert.match(markPendingText(pendingBefore.items), /Cut the legacy export/, "mark --pending lists decision 7 before the archive");

  // The archive writes its own record and never a status into the decision row: the decision stays pending.
  const r = await cli("archive", "7", "--product", "agentsi");
  assert.equal(r.status, 0, r.stderr);

  const statusAfter = await core.statusMarkdown("agentsi", db, h.home);
  assert.doesNotMatch(statusAfter, /Cut the legacy export/, "status no longer lists decision 7");
  assert.match(statusAfter, /\[8\] Ship the pricing page/, "status still lists the other pending decision");
  assert.match(statusAfter, /Pending decisions: 1/, "status counts only the decision that is not archived");

  const pendingAfter = await core.pendingItems("agentsi", { db, root: h.home });
  assert.deepEqual(
    pendingAfter.items.map((i) => (i.kind === "decision" ? i.id : null)),
    [8],
    "mark --pending lists only decision 8",
  );
  assert.doesNotMatch(markPendingText(pendingAfter.items), /Cut the legacy export/, "mark --pending no longer prints decision 7");
});

test("agentsi decide refuses an archived decision, says to restore it first, and changes nothing", async (t) => {
  const testDatabase = await testDb();
  t.after(() => testDatabase.close());
  assert.equal((await cli("archive", "9", "--product", "agentsi")).status, 0, "decision 9 is archived first");
  const before = snapshot(h.home);

  for (const verdict of ["approve", "reject"]) {
    const r = await cli("decide", "9", verdict);
    assert.notEqual(r.status, 0, `decide ${verdict} exits with an error`);
    assert.equal(r.stdout, "", `decide ${verdict} prints no decision outcome`);
    assert.match(r.stderr, /archiv/i, `decide ${verdict} says the decision is archived`);
    assert.match(r.stderr, /restore/i, `decide ${verdict} says to restore it first`);
  }

  assert.deepEqual(differing(before, snapshot(h.home)), [], "the refused decide leaves the data root as it was");
});
