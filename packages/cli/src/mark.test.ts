import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { PassThrough, Writable } from "node:stream";
import { after, test, type TestContext } from "node:test";
import { fileURLToPath } from "node:url";
import { getPool, pendingItems, type DecisionStore, type MarkEntry, type PendingItem } from "@agentsi/core";
import { testDb } from "@agentsi/core/test-db";
import { testHome } from "./test-home.ts";
import { parseSet } from "./commands/mark.ts";
import { runWizard, type WizardDeps } from "./commands/mark-wizard.ts";

const bin = fileURLToPath(new URL("../bin/agentsi.mjs", import.meta.url));
const h = testHome([{ id: "agentsi", name: "AgentSI", roles: ["pm", "dev"] }]);
after(h.cleanup);
const run = (...args: string[]) => spawnSync(process.execPath, [bin, ...args], { encoding: "utf8", env: h.env, input: "" });

const items: PendingItem[] = [
  { kind: "priority", minutes_id: "m1", idx: 0, title: "Ship A", why: "because", owner_role: "pm", objective: "o1", sources: [{ type: "path", ref: "a.md" }] },
  { kind: "priority", minutes_id: "m1", idx: 1, title: "Ship B", why: "w", owner_role: "dev", objective: null, sources: [] },
  { kind: "priority", minutes_id: "m1", idx: 2, title: "Ship C", why: "w", owner_role: "qa", objective: null, sources: [] },
  { kind: "decision", id: 7, minutes_id: "m1", text: "Cut X", why: "cost", action: "Delete X" },
  { kind: "decision", id: 8, minutes_id: null, text: "Cut Y", why: null, action: null },
];

function harness(answers: string[], decideStatus: "approved" | "rejected" | "stale" = "approved") {
  const marks: MarkEntry[][] = [];
  const decided: [number, string][] = [];
  const deps: WizardDeps = {
    async applyMarks(e) { marks.push(e); return { committed: true }; },
    async decide(id, status) { decided.push([id, status]); return { status: decideStatus === "stale" ? "stale" : status, minutesId: "m1", action: id === 7 ? "Delete X" : null, commit: { committed: true } }; },
  };
  const input = new PassThrough();
  const sink = () => {
    let text = "";
    return { stream: new Writable({ write(chunk, _enc, done) { text += chunk; done(); } }), text: () => text };
  };
  const output = sink();
  const error = sink();
  input.end(answers.join("\n") + "\n");
  return { marks, decided, deps, io: { input, output: output.stream, error: error.stream }, out: output.text, err: error.text };
}

test("wizard applies marks in one batch and decisions one by one, and prints the action", async () => {
  const h = harness(["a", "", "c", "y", "s", "a", "r"]);
  const code = await runWizard(items, h.io, h.deps);
  assert.equal(code, 0);
  assert.deepEqual(h.marks, [[
    { minutesId: "m1", idx: 0, mark: "accepted", isNew: false },
    { minutesId: "m1", idx: 1, mark: "changed", isNew: true },
  ]]);
  assert.deepEqual(h.decided, [[7, "approved"], [8, "rejected"]]);
  const out = h.out();
  assert.match(out, /3 priorities and 2 decisions pending\./);
  assert.match(out, /Action: Delete X/);
  assert.match(out, /Marked 2, approved 1, rejected 1, skipped 1\./);
});

test("wizard asks again on a bad key", async () => {
  const h = harness(["zz", "r", "n"]);
  await runWizard(items.slice(0, 1), h.io, h.deps);
  assert.deepEqual(h.marks[0], [{ minutesId: "m1", idx: 0, mark: "rejected", isNew: false }]);
});

test("wizard applies the answers given before q", async () => {
  const h = harness(["a", "n", "q"]);
  await runWizard(items, h.io, h.deps);
  assert.equal(h.marks[0]!.length, 1);
  assert.deepEqual(h.decided, []);
  assert.match(h.out(), /Marked 1, approved 0, rejected 0, skipped 0\./);
});

test("wizard applies the answers given when the input ends", async () => {
  const h = harness(["a", "n"]);
  await runWizard(items, h.io, h.deps);
  assert.equal(h.marks[0]!.length, 1);
});

test("wizard stops without the mark when the input ends at the new question", async () => {
  const h = harness(["a"]);
  await runWizard(items, h.io, h.deps);
  assert.deepEqual(h.marks, []);
  assert.match(h.out(), /Marked 0, approved 0, rejected 0, skipped 0\./);
});

test("wizard says when nothing is pending and asks nothing", async () => {
  const h = harness([]);
  assert.equal(await runWizard([], h.io, h.deps), 0);
  assert.match(h.out(), /^Nothing pending\.\n$/);
});

test("a stale decision fails the run but the other decisions go on", async () => {
  const h = harness(["a", "a"], "stale");
  const code = await runWizard(items.slice(3), h.io, h.deps);
  assert.equal(code, 1);
  assert.equal(h.decided.length, 2);
  assert.match(h.err(), /Decision 7 not applied: .* stale/);
});

test("parseSet reads the optional new flag and rejects bad specs", () => {
  assert.deepEqual(parseSet("2026-10-17-1336-x:3:rejected:new"), { minutesId: "2026-10-17-1336-x", idx: 3, mark: "rejected", isNew: true });
  assert.deepEqual(parseSet("m:0:accepted"), { minutesId: "m", idx: 0, mark: "accepted", isNew: false });
  for (const bad of ["m:0", "m:x:accepted", "m:0:ok", "m:0:accepted:old", ":0:accepted", "m:0:accepted:new:more"]) assert.throws(() => parseSet(bad), /Bad --set/);
});

test("mark with no arguments and no terminal prints the usage and exits 1", () => {
  const r = run("mark", "--product", "agentsi");
  assert.equal(r.status, 1);
  assert.match(r.stderr, /agentsi mark --pending/);
});

test("a bad --set writes nothing and exits 1", () => {
  const r = run("mark", "--set", "m:0:maybe");
  assert.equal(r.status, 1);
  assert.match(r.stderr, /Bad --set/);
});

test("mark --pending --json lists priorities and reports the database error on stderr only", () => {
  const r = run("mark", "--pending", "--json", "--product", "agentsi");
  assert.equal(r.status, 0, r.stderr);
  assert.ok(Array.isArray(JSON.parse(r.stdout)));
  assert.match(r.stderr, /^decisions_error: /);
});

test("the three-argument form still validates its arguments", () => {
  assert.equal(run("mark", "m1", "x", "accepted").status, 1);
  assert.equal(run("mark", "m1", "0", "maybe").status, 1);
});

// S007: approving a decision marks the priorities it links. The owner meets them through the wizard,
// so these tests read the pending list from real minutes rows and check what the wizard asks to apply.

/** One product, minutes m1 in a test database. Decisions carry the indexes of the priorities they cover. */
async function minutesInDb(t: TestContext, priorities: object[], decisions: object[]): Promise<string> {
  const db = await testDb();
  t.after(() => db.close());
  const m = {
    id: "m1", product: "agentsi", scope: null, question: "q", created_at: "2026-10-17T00:00:00Z", roles: {},
    priorities, disagreements: [], decisions, script: [], check: { passed: true, failures: [], repaired: false }, status: "valid",
  };
  await getPool().query("insert into minutes (id, product, data) values ('m1', 'agentsi', $1::jsonb)", [JSON.stringify(m)]);
  return mkdtempSync(join(tmpdir(), "s007-marks-"));
}

const unmarked = (title: string, mark: "rejected" | null = null) => ({ title, why: "w", sources: [], owner_role: "pm", objective: null, mark, is_new: false });
const linkedDecision = (priorities: number[]) => ({ text: "Cut X", why: "cost", action: "Delete X", status: "pending", priorities });
const pendingDecisionRow = { id: "7", minutes_id: "m1", text: "Cut X", why: "cost", action: "Delete X" };

async function pendingIn(root: string): Promise<PendingItem[]> {
  const store: DecisionStore = { async query<R extends object>() { return { rows: [pendingDecisionRow] as unknown as R[] }; } };
  const { items, decisions_error } = await pendingItems("agentsi", { root, db: store });
  assert.equal(decisions_error, undefined);
  return items;
}

/** Every mark the wizard asked to apply, in priority order. */
const appliedMarks = (marks: MarkEntry[][]) => marks.flat().sort((a, b) => a.idx - b.idx);

test("approving a decision marks accepted each unmarked priority it links, and no other", async (t) => {
  const root = await minutesInDb(t,[unmarked("A"), unmarked("B"), unmarked("C"), unmarked("D")], [linkedDecision([0, 1, 2])]);
  const h = harness(["s", "s", "s", "s", "a"]);
  assert.equal(await runWizard(await pendingIn(root), h.io, h.deps), 0);
  assert.deepEqual(appliedMarks(h.marks), [
    { minutesId: "m1", idx: 0, mark: "accepted", isNew: false },
    { minutesId: "m1", idx: 1, mark: "accepted", isNew: false },
    { minutesId: "m1", idx: 2, mark: "accepted", isNew: false },
  ]);
  assert.deepEqual(h.decided, [[7, "approved"]]);
});

test("approving a decision never overwrites a priority that already has a mark", async (t) => {
  const root = await minutesInDb(t,[unmarked("A", "rejected"), unmarked("B")], [linkedDecision([0, 1])]);
  const h = harness(["s", "a"]);
  assert.equal(await runWizard(await pendingIn(root), h.io, h.deps), 0);
  assert.deepEqual(appliedMarks(h.marks), [{ minutesId: "m1", idx: 1, mark: "accepted", isNew: false }]);
  assert.deepEqual(h.decided, [[7, "approved"]]);
});

test("rejecting a decision marks none of the priorities it links and shows the decision as rejected", async (t) => {
  const root = await minutesInDb(t,[unmarked("A"), unmarked("B"), unmarked("C")], [linkedDecision([0, 1, 2])]);
  const h = harness(["s", "s", "s", "r"]);
  assert.equal(await runWizard(await pendingIn(root), h.io, h.deps), 0);
  assert.deepEqual(appliedMarks(h.marks), []);
  assert.deepEqual(h.decided, [[7, "rejected"]]);
  assert.match(h.out(), /Decision 7 rejected \(m1\)\./);
});
