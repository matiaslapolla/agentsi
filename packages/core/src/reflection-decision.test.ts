import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { git } from "./exec.ts";
import { sha256 } from "./meeting-log.ts";
import { readReflections, writeReflections, type Proposal, type ReflectionEntry } from "./reflect.ts";
import { decide } from "./minutes.ts";
import { applyProposal, settleReflection, targetHash, type ApplyProposal, type DecisionStore } from "./reflection-decision.ts";
import { readOverlay, writeOverlay } from "./roster.ts";
import { testDb } from "./test-db.ts";

const TARGET = "workspace/demo/agents/dev.md";
const OVERLAY = "---\nextends: dev\n---\nCite the plan.\n";
const NEW_OVERLAY = "---\nextends: dev\n---\nCite the plan. Cite only paths you read.\n";

const pendingProposal = (over: Partial<Proposal> = {}): Proposal => ({
  agent: "dev",
  target: TARGET,
  trigger: { reason: "failure_class", failure_class: "unresolved_source", count: 3 },
  shared_lesson: null,
  rationale: "cites paths it never read",
  expected_effect: "fewer unresolved sources",
  diff: "",
  content: NEW_OVERLAY,
  body_hash: "h",
  target_hash: sha256(OVERLAY),
  replay: { tuning: [], held_out: [], rows: [], totals: null, verdict: { passed: true, worse: [], better: [] }, cost_rule: "", cost_usd: 0, budget_usd: 15, aborted: null },
  decision_id: 7,
  status: "pending",
  ...over,
});

async function fixture(proposal: Partial<Proposal> = {}) {
  const db = await testDb();
  const root = mkdtempSync(join(tmpdir(), "agentsi-settle-"));
  await writeOverlay("demo", "dev", OVERLAY);
  const entry: ReflectionEntry = {
    id: "r-1",
    product: "demo",
    created_at: "2026-10-13T15:30:00.000Z",
    trigger: "manual",
    signals: [],
    memory: [],
    proposal: pendingProposal(proposal),
    status: "applied",
    commit: null,
  };
  await writeReflections("demo", [entry], root);
  await git(root, "init", "-q");
  await git(root, "config", "user.email", "t@example.com");
  await git(root, "config", "user.name", "T");
  await git(root, "add", ".");
  await git(root, "commit", "-qm", "init");
  return {
    root,
    cleanup: async () => {
      rmSync(root, { recursive: true, force: true });
      await db.close();
    },
  };
}

function fakeApply() {
  const calls: string[] = [];
  const apply: ApplyProposal = async (id) => {
    calls.push(id);
    return { committed: true };
  };
  return { calls, apply };
}

const proposalStatus = async (root: string) => (await readReflections("demo", root))[0]!.proposal!.status;
const count = async (root: string) => Number((await git(root, "rev-list", "--count", "HEAD")).stdout.trim());

test("approving applies the proposal when the target still has its hash", async () => {
  const f = await fixture();
  try {
    const fake = fakeApply();
    const res = await settleReflection("demo", "r-1", "approved", { root: f.root, apply: fake.apply });
    assert.deepEqual(res, { status: "approved", commit: { committed: true } });
    assert.deepEqual(fake.calls, ["r-1"]);
  } finally {
    await f.cleanup();
  }
});

test("approving refuses and marks the proposal stale when the target changed since the proposal", async () => {
  const f = await fixture();
  try {
    await writeOverlay("demo", "dev", OVERLAY + "Edited by hand.\n");
    const before = await count(f.root);
    const fake = fakeApply();
    const res = await settleReflection("demo", "r-1", "approved", { root: f.root, apply: fake.apply });
    assert.equal(res.status, "stale");
    assert.equal(res.commit.committed, false);
    assert.equal(await count(f.root), before, "the data root gets no commit");
    assert.deepEqual(fake.calls, []);
    assert.equal(await proposalStatus(f.root), "stale");
    assert.deepEqual(await settleReflection("demo", "r-1", "approved", { root: f.root, apply: fake.apply }), { status: "stale", commit: { committed: false } });
    await assert.rejects(settleReflection("demo", "r-1", "rejected", { root: f.root, apply: fake.apply }), /already stale/);
  } finally {
    await f.cleanup();
  }
});

test("a proposal for a new overlay is stale once the overlay appears", async () => {
  const f = await fixture({ target: "workspace/demo/agents/qa.md", target_hash: null });
  try {
    assert.equal(await targetHash("workspace/demo/agents/qa.md", f.root), null);
    await writeOverlay("demo", "qa", "x\n");
    const res = await settleReflection("demo", "r-1", "approved", { root: f.root, apply: fakeApply().apply });
    assert.equal(res.status, "stale");
  } finally {
    await f.cleanup();
  }
});

test("rejecting records it on the proposal without applying", async () => {
  const f = await fixture();
  try {
    const fake = fakeApply();
    const res = await settleReflection("demo", "r-1", "rejected", { root: f.root, apply: fake.apply });
    assert.equal(res.status, "rejected");
    assert.deepEqual(fake.calls, []);
    assert.equal(await proposalStatus(f.root), "rejected");
  } finally {
    await f.cleanup();
  }
});

test("settling refuses a reflection without a proposal", async () => {
  const f = await fixture();
  try {
    await assert.rejects(settleReflection("demo", "r-nope", "approved", { root: f.root }), /has no proposal/);
  } finally {
    await f.cleanup();
  }
});

test("settling again with the same verdict returns the earlier outcome without a second commit", async () => {
  const f = await fixture();
  try {
    await settleReflection("demo", "r-1", "rejected", { root: f.root });
    const before = await count(f.root);
    assert.deepEqual(await settleReflection("demo", "r-1", "rejected", { root: f.root }), { status: "rejected", commit: { committed: false } });
    assert.equal(await count(f.root), before);
    await assert.rejects(settleReflection("demo", "r-1", "approved", { root: f.root }), /already rejected/);
  } finally {
    await f.cleanup();
  }
});

/** A decisions table with one reflection decision; `failUpdates` makes that many status updates throw. */
function decisionsTable(reflectionId: string, failUpdates = 0) {
  const updates: unknown[][] = [];
  const db: DecisionStore = {
    async query<R extends object>(sql: string, params: unknown[]) {
      if (sql.startsWith("select")) return { rows: [{ product: "demo", kind: "reflection", reflection_id: reflectionId }] as unknown as R[] };
      if (failUpdates-- > 0) throw new Error("connection reset");
      updates.push([sql, ...params]);
      return { rows: [] };
    },
  };
  return { db, updates };
}

test("decide retried after a failed status update settles idempotently and records the status", async () => {
  const f = await fixture();
  try {
    const t = decisionsTable("r-1", 1);
    await assert.rejects(decide(7, "approved", { db: t.db, root: f.root }), /connection reset/);
    assert.equal(await proposalStatus(f.root), "approved");
    const before = await count(f.root);
    const res = await decide(7, "approved", { db: t.db, root: f.root });
    assert.equal(res.status, "approved");
    assert.equal(res.action, null);
    assert.equal(await count(f.root), before);
    assert.deepEqual(t.updates.map((u) => u.slice(1)), [[7, "approved"]]);
  } finally {
    await f.cleanup();
  }
});

test("decide rejects a reflection decision whose entry is missing, with a clear message", async () => {
  const f = await fixture();
  try {
    const t = decisionsTable("r-gone");
    await assert.rejects(decide(7, "approved", { db: t.db, root: f.root }), /Decision 7 rejected: Reflection "r-gone" has no proposal in demo\. It can never be applied\./);
    assert.equal(t.updates.length, 1);
    assert.match(t.updates[0]![0] as string, /set status = 'rejected'/);
  } finally {
    await f.cleanup();
  }
});

test("decide on a reflection decision rejects the proposal and records rejected without touching minutes", async () => {
  const f = await fixture();
  try {
    const t = decisionsTable("r-1");
    const res = await decide(7, "rejected", { db: t.db, root: f.root });
    assert.deepEqual({ status: res.status, minutesId: res.minutesId, action: res.action }, { status: "rejected", minutesId: null, action: null });
    assert.equal(await proposalStatus(f.root), "rejected");
    assert.equal(t.updates.length, 1);
    assert.match(t.updates[0]![0] as string, /^update decisions set status = \$2, decided_at = now\(\) where id = \$1$/);
    assert.deepEqual(t.updates[0]!.slice(1), [7, "rejected"]);
  } finally {
    await f.cleanup();
  }
});

test("decide records stale, not approved, when the target changed since the proposal", async () => {
  const f = await fixture();
  try {
    await writeOverlay("demo", "dev", OVERLAY + "Edited by hand.\n");
    const t = decisionsTable("r-1");
    const res = await decide(7, "approved", { db: t.db, root: f.root });
    assert.equal(res.status, "stale");
    assert.equal(await readOverlay("demo", "dev"), OVERLAY + "Edited by hand.\n");
    assert.deepEqual(t.updates.map((u) => u.slice(1)), [[7, "stale"]]);
  } finally {
    await f.cleanup();
  }
});

test("decide on a reflection decision with no reflection id fails before any write", async () => {
  const updates: string[] = [];
  const db: DecisionStore = {
    async query<R extends object>(sql: string) {
      if (sql.startsWith("select")) return { rows: [{ product: "demo", kind: "reflection", reflection_id: null }] as unknown as R[] };
      updates.push(sql);
      return { rows: [] };
    },
  };
  await assert.rejects(decide(7, "approved", { db }), /reflection decision without a reflection id/);
  assert.deepEqual(updates, []);
});

test("decide refuses a decision that does not exist", async () => {
  const db: DecisionStore = { query: async () => ({ rows: [] }) };
  await assert.rejects(decide(99, "approved", { db }), /Decision 99 not found/);
});

test("applyProposal stores the content in the database, marks the proposal approved and makes no commit", async () => {
  const f = await fixture();
  try {
    const before = await count(f.root);
    const res = await settleReflection("demo", "r-1", "approved", { root: f.root });
    assert.deepEqual(res, { status: "approved", commit: { committed: false } });
    assert.equal(await readOverlay("demo", "dev"), NEW_OVERLAY);
    assert.equal(await proposalStatus(f.root), "approved");
    assert.equal(await count(f.root), before, "the data root gets no commit");
    assert.equal((await git(f.root, "status", "--porcelain")).stdout.trim(), "");
    await assert.rejects(applyProposal("r-1", { product: "demo", root: f.root }), /already approved/);
  } finally {
    await f.cleanup();
  }
});

test("applyProposal refuses a changed target and a target outside the agent's overlay or the global roles", async () => {
  const changed = await fixture();
  try {
    await writeOverlay("demo", "dev", "edited\n");
    await assert.rejects(applyProposal("r-1", { product: "demo", root: changed.root }), /changed since the proposal/);
    assert.equal(await readOverlay("demo", "dev"), "edited\n");
  } finally {
    await changed.cleanup();
  }
  const outside = await fixture({ target: "workspace/demo/memory/dev.json", target_hash: null });
  try {
    await assert.rejects(applyProposal("r-1", { product: "demo", root: outside.root }), /may not write/);
  } finally {
    await outside.cleanup();
  }
});
