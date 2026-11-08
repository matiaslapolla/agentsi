import assert from "node:assert/strict";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { CheckOptions } from "./check.ts";
import type { ClaudeCall } from "./claude.ts";
import { getPool } from "./db.ts";
import { git } from "./exec.ts";
import { roleAnswerSchema } from "./meeting.ts";
import { sha256, type MeetingLogEntry } from "./meeting-log.ts";
import { writeMemoryItems, type MemoryItem } from "./memory.ts";
import {
  PROPOSAL_MODEL,
  chooseTarget,
  normalizeLesson,
  proposalSystem,
  proposeForReady,
  renderProposal,
  type ProposalTarget,
} from "./proposal.ts";
import { DELTA_SCHEMA, readReflections, reflect, type ReflectionEntry } from "./reflect.ts";
import { recordReflectionDecision, settleReflection, type DecisionStore } from "./reflection-decision.ts";
import type { Product } from "./registry.ts";
import { loadRole, roleDirsAt } from "./roles.ts";
import { readOverlay, writeOverlay } from "./roster.ts";
import { testDb } from "./test-db.ts";
import type { Minutes, Source } from "./types.ts";

const GLOBAL_DEV = "---\nname: dev\nmodel: sonnet\neffort: medium\ntools: Read, Grep, Glob\n---\n\nYou are the dev.\n";
const GLOBAL_PM = "---\nname: pm\nmodel: sonnet\n---\n\nYou are the pm.\n";
const RULE = "Cite only paths you read.";
const NOW = new Date("2026-10-13T15:30:00.000Z");
/** By the parity of sha256(id): days 01 and 03 are tuning, 02 and 04 are held out. */
const day = (d: number) => `2026-10-0${d}-1200-q`;

const lesson = (id: string, text: string): MemoryItem => ({
  id,
  text,
  kind: "lesson",
  helpful: 0,
  harmful: 0,
  source: "r-0",
  created_at: "2026-09-01T00:00:00.000Z",
  retired_at: null,
  retired_why: null,
});

const usage = (cost: number) => ({ model: "m", input_tokens: 0, output_tokens: 0, cache_read: 0, cache_write: 0, cost_usd: cost, duration_ms: 1 });

const path = (ref: string): Source => ({ type: "path", ref });

function logEntry(id: string, devHash: string): MeetingLogEntry {
  const run = (prompt_hash: string, unresolved: Source[]) => ({
    prompt_hash,
    memory_hash: "",
    model: "sonnet",
    cost_usd: 0.01,
    duration_ms: 1,
    error: null,
    priorities: 1,
    sources: 1,
    unresolved_sources: unresolved,
    sin_datos: 0,
  });
  return {
    minutes_id: id,
    created_at: `${id.slice(0, 10)}T12:00:00.000Z`,
    question: "¿Qué sigue?",
    scope: null,
    brief_hash: null,
    agents: { pm: run(sha256("You are the pm."), []), dev: run(devHash, [path("bad.md")]) },
    synthesis: {
      first_check: { passed: false, failures: ['Priority #0: path source "bad.md" does not resolve.'] },
      repaired: true,
      final_check: { passed: true, failures: [] },
      invented_sources: [],
      cost_usd: 0.05,
    },
    total_cost_usd: 0.07,
  };
}

function minutesFile(id: string): Minutes {
  const answer = (text: string, ref: string) => ({ model: "sonnet", answer: text, priorities: [{ title: "t", why: "w", sources: [path(ref)] }], concerns: [], memory_notes: [] });
  return {
    id,
    product: "demo",
    scope: null,
    question: "¿Qué sigue?",
    created_at: `${id.slice(0, 10)}T12:00:00.000Z`,
    roles: { pm: answer("pm view", "good.md"), dev: answer("dev view", "bad.md") },
    priorities: [{ title: "t", why: "w", sources: [path("good.md")], owner_role: "dev", objective: null, mark: null, is_new: false }],
    disagreements: [],
    decisions: [],
    script: [{ role: "pm", text: "hola" }],
    check: { passed: true, failures: [], first_failures: [], repaired: true },
    status: "valid",
  };
}

interface FixtureOptions {
  days?: number;
  devHash?: string;
  memory?: Record<string, Record<string, MemoryItem[]>>;
}

async function fixture(o: FixtureOptions = {}) {
  const db = await testDb();
  const root = mkdtempSync(join(tmpdir(), "agentsi-proposal-"));
  const repo = join(root, "repo");
  mkdirSync(join(repo, "docs"), { recursive: true });
  writeFileSync(join(repo, "good.md"), "x\n");
  mkdirSync(join(root, "plugin/agents"), { recursive: true });
  writeFileSync(join(root, "plugin/agents/dev.md"), GLOBAL_DEV);
  writeFileSync(join(root, "plugin/agents/pm.md"), GLOBAL_PM);
  const ws = join(root, "workspace/demo");
  mkdirSync(ws, { recursive: true });
  const hash = o.devHash ?? sha256("You are the dev.");
  const log: MeetingLogEntry[] = [];
  for (let d = 1; d <= (o.days ?? 4); d++) {
    const entry = logEntry(day(d), hash);
    log.push(entry);
    await getPool().query("insert into minutes (id, product, data) values ($1, 'demo', $2::jsonb)", [day(d), JSON.stringify(minutesFile(day(d)))]);
    await getPool().query("insert into snapshots (product, minutes_id, body) values ('demo', $1, '# Snapshot\n')", [day(d)]);
    await getPool().query("insert into meeting_log (product, minutes_id, entry) values ('demo', $1, $2::jsonb)", [day(d), JSON.stringify(entry)]);
  }
  for (const [product, agents] of Object.entries(o.memory ?? {})) {
    for (const [agent, items] of Object.entries(agents)) await writeMemoryItems(product, agent, items);
  }
  await git(root, "init", "-q");
  await git(root, "config", "user.email", "t@example.com");
  await git(root, "config", "user.name", "T");
  await git(root, "add", ".");
  await git(root, "commit", "-qm", "init");
  const product: Product = { id: "demo", name: "Demo", paths: [repo], status: "active", roles: ["pm", "dev"] };
  return {
    root,
    ws,
    product,
    log,
    cleanup: async () => {
      rmSync(root, { recursive: true, force: true });
      await db.close();
    },
  };
}

/**
 * A stubbed claude wrapper. Curation returns no deltas. The proposal returns `body`. An agent whose prompt holds RULE
 * cites `candidate`, any other cites bad.md. The synthesis cites every path the agent answers cite.
 */
function stub(o: { body?: string; candidate?: string[] } = {}) {
  const calls: ClaudeCall[] = [];
  const call = async (c: ClaudeCall) => {
    calls.push(c);
    const props = (c.schema as { properties?: Record<string, unknown> } | undefined)?.properties ?? {};
    if (c.schema === DELTA_SCHEMA) return { output: { add: [], retire: [], helpful: [], harmful: [] }, usage: usage(0.001) };
    if (c.model === PROPOSAL_MODEL && ("new_overlay_body" in props || "new_global_body" in props)) {
      const key = "new_overlay_body" in props ? "new_overlay_body" : "new_global_body";
      return { output: { [key]: o.body ?? RULE, rationale: `cites paths it never read (${day(1)})`, expected_effect: "fewer unresolved sources" }, usage: usage(0.05) };
    }
    if (c.schema === roleAnswerSchema) {
      const refs = c.system.includes(RULE) ? (o.candidate ?? ["good.md"]) : ["bad.md"];
      return { output: { answer: "dev view", priorities: [{ title: "t", why: "w", sources: refs.map(path) }], concerns: [], memory_notes: [] }, usage: usage(0.01) };
    }
    const refs = [...new Set([...c.prompt.matchAll(/"ref": "([^"]+)"/g)].map((m) => m[1]!))];
    return {
      output: { priorities: [{ title: "t", why: "w", sources: refs.map(path), owner_role: "dev" }], disagreements: [], decisions: [], script: [{ role: "pm", text: "hola" }] },
      usage: usage(0.05),
    };
  };
  return { calls, call };
}

const checkStub = async (m: Minutes, _o: CheckOptions) => {
  const failures = m.priorities.flatMap((p, i) => p.sources.filter((s) => s.ref !== "good.md").map((s) => `Priority #${i}: path source "${s.ref}" does not resolve.`));
  return { passed: failures.length === 0, failures };
};

function store() {
  const inserts: { sql: string; params: unknown[] }[] = [];
  const db: DecisionStore = {
    async query<R extends object>(sql: string, params: unknown[]) {
      inserts.push({ sql, params });
      return { rows: [{ id: "12" }] as unknown as R[] };
    },
  };
  const recordDecision: typeof recordReflectionDecision = (p, r, t, w) => recordReflectionDecision(p, r, t, w, db);
  return { inserts, recordDecision };
}

const deps = (s: ReturnType<typeof stub>, rec = store()) => ({
  call: s.call,
  check: checkStub,
  resolve: async (src: Source) => src.ref === "good.md",
  recordDecision: rec.recordDecision,
});

const quiet = () => {};
const count = async (root: string) => Number((await git(root, "rev-list", "--count", "HEAD")).stdout.trim());

test("below 4 replay cases reflect makes no prompt proposal and says why", async () => {
  const f = await fixture({ days: 3 });
  try {
    const s = stub();
    const { entry, candidates } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(s) });
    assert.deepEqual(candidates.map((c) => [c.agent, c.outcome, c.cases]), [["dev", "insufficient_evidence", 3]]);
    assert.match(candidates[0]!.note!, /3 casos de replay \(mínimo 4\)/);
    assert.equal(entry!.proposal, null);
    assert.deepEqual(entry!.candidates, candidates);
    assert.ok(s.calls.every((c) => c.schema === DELTA_SCHEMA), "only curation calls");
  } finally {
    await f.cleanup();
  }
});

test("with proposals off, reflect only curates memory", async () => {
  const f = await fixture();
  try {
    const s = stub();
    const { entry, candidates } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, proposals: false, ...deps(s) });
    assert.deepEqual(candidates, []);
    assert.equal(entry!.proposal, null);
    assert.ok(s.calls.every((c) => c.schema === DELTA_SCHEMA));
  } finally {
    await f.cleanup();
  }
});

test("a proposal that worsens a held-out metric is discarded and records no decision", async () => {
  const f = await fixture();
  try {
    const s = stub({ candidate: ["bad.md", "worse.md"] });
    const rec = store();
    const before = await count(f.root);
    const { entry } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(s, rec) });
    const p = entry!.proposal!;
    assert.equal(p.status, "discarded");
    assert.deepEqual(p.replay.verdict.worse, ["unresolved_per_case"]);
    assert.deepEqual(p.replay.held_out, [day(2), day(4)]);
    assert.deepEqual(p.replay.tuning, [day(1), day(3)]);
    assert.equal(p.replay.totals!.before.unresolved_per_case, 1);
    assert.equal(p.replay.totals!.after.unresolved_per_case, 2);
    assert.equal(p.decision_id, null);
    assert.deepEqual(rec.inserts, []);
    assert.equal(await count(f.root), before, "the data root gets no commit");
    assert.equal(await readOverlay("demo", "dev"), null, "replay writes no overlay");
    assert.equal((await getPool().query("select 1 from minutes")).rows.length, 4, "replay writes no minutes");
    assert.equal((await getPool().query("select 1 from meeting_log")).rows.length, 4, "nor meeting log lines");
  } finally {
    await f.cleanup();
  }
});

test("a proposal that improves without worsening becomes a pending reflection decision", async () => {
  const f = await fixture();
  try {
    const s = stub();
    const rec = store();
    const { entry, commit } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(s, rec) });
    assert.equal(commit.committed, false);
    const p = entry!.proposal!;
    assert.equal(p.status, "pending");
    assert.equal(p.decision_id, 12);
    assert.equal(p.target, "workspace/demo/agents/dev.md");
    assert.equal(p.target_hash, null);
    assert.equal(p.content, `---\nextends: dev\n---\n${RULE}\n`);
    assert.match(p.diff, /^\+Cite only paths you read\.$/m);
    assert.deepEqual(p.replay.verdict, { passed: true, worse: [], better: ["first_pass_rate", "unresolved_per_case"] });
    assert.deepEqual(p.replay.rows.map((r) => r.baseline), ["stored", "stored"]);
    assert.deepEqual(entry!.candidates!.map((c) => c.outcome), ["proposed"]);

    const [insert] = rec.inserts;
    assert.match(insert!.sql, /insert into decisions \(product, kind, reflection_id, text, why\) values \(\$1, 'reflection'/);
    assert.deepEqual(insert!.params.slice(0, 2), ["demo", entry!.id]);
    const text = insert!.params[2] as string;
    assert.match(text, /^Cambiar el prompt de dev \(workspace\/demo\/agents\/dev\.md, overlay nuevo\)/);
    assert.match(text, new RegExp(`^- ${day(2)}: check falla, sin resolver 1, .* → check ok, sin resolver 0, `, "m"));
    assert.match(text, /^Totales: check al primer intento 0% → 100%, sin resolver por caso 1\.00 → 0\.00/m);
    assert.equal(insert!.params[3], p.rationale);

    const proposalCall = s.calls.find((c) => c.model === PROPOSAL_MODEL)!;
    assert.equal(proposalCall.effort, "high");
    assert.deepEqual(proposalCall.tools, []);
    assert.match(proposalCall.prompt, new RegExp(`<case minutes="${day(1)}">`));
    assert.doesNotMatch(proposalCall.prompt, new RegExp(day(2)), "the proposal never sees a held-out case");
    assert.match(proposalCall.prompt, /<effective_prompt>\nYou are the dev\.\n<\/effective_prompt>/);
    assert.ok(s.calls.every((c) => (c.maxBudgetUsd ?? 0) <= 1.5 || c.schema === DELTA_SCHEMA));
    assert.ok(entry!.cost_usd! > 0.05);
  } finally {
    await f.cleanup();
  }
});

test("the entry is on disk with a pending proposal before its decision is recorded", async () => {
  const f = await fixture();
  try {
    const seen: (ReflectionEntry | undefined)[] = [];
    const recordDecision: typeof recordReflectionDecision = async (product, reflectionId) => {
      seen.push((await readReflections(product, f.root)).find((e) => e.id === reflectionId));
      return 12;
    };
    const { entry } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(stub()), recordDecision });
    assert.equal(seen.length, 1);
    assert.deepEqual([seen[0]!.proposal!.status, seen[0]!.proposal!.decision_id], ["pending", null]);
    assert.equal(entry!.proposal!.decision_id, 12);
    assert.deepEqual(await readReflections("demo", f.root), [entry]);
  } finally {
    await f.cleanup();
  }
});

test("a failed decision insert discards the proposal with the reason and keeps the reflection", async () => {
  const f = await fixture();
  try {
    const before = await count(f.root);
    const recordDecision = async () => Promise.reject(new Error("db down"));
    const { entry, commit } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(stub()), recordDecision });
    assert.equal(commit.committed, false);
    assert.equal(await count(f.root), before, "the data root gets no commit");
    assert.deepEqual([entry!.proposal!.status, entry!.proposal!.decision_id], ["discarded", null]);
    assert.equal(entry!.proposal!.replay.aborted, "no se pudo registrar la decisión: db down");
    assert.deepEqual(await readReflections("demo", f.root), [entry]);
  } finally {
    await f.cleanup();
  }
});

test("approving the decision stores the overlay in the database and commits the reflection; a target changed since the proposal is stale", async () => {
  const f = await fixture();
  try {
    const { entry } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(stub()) });
    const before = await count(f.root);
    const res = await settleReflection("demo", entry!.id, "approved", { root: f.root });
    assert.deepEqual(res, { status: "approved", commit: { committed: false } });
    assert.equal(await count(f.root), before, "the data root gets no commit");
    assert.equal((await git(f.root, "status", "--porcelain")).stdout.trim(), "");
    assert.equal(await readOverlay("demo", "dev"), `---\nextends: dev\n---\n${RULE}\n`);
    assert.equal(existsSync(join(f.ws, "agents")), false, "no overlay file");
    assert.match((await loadRole("dev", "demo", roleDirsAt(f.root))).body, /## This product\n\nCite only paths you read\./);
    assert.equal((await readReflections("demo", f.root)).at(-1)!.proposal!.status, "approved");
  } finally {
    await f.cleanup();
  }

  const g = await fixture();
  try {
    const { entry } = await reflect(g.product, { root: g.root, now: NOW, warn: quiet, ...deps(stub()) });
    await writeOverlay("demo", "dev", "---\nextends: dev\n---\nWritten by hand.\n");
    const res = await settleReflection("demo", entry!.id, "approved", { root: g.root });
    assert.equal(res.status, "stale");
    assert.equal(await readOverlay("demo", "dev"), "---\nextends: dev\n---\nWritten by hand.\n");
    assert.equal((await readReflections("demo", g.root)).at(-1)!.proposal!.status, "stale");
  } finally {
    await g.cleanup();
  }
});

test("a lesson held in two products targets the global role, and approval commits plugin/agents in the same commit", async () => {
  const f = await fixture({
    memory: {
      demo: { dev: [lesson("m-aaaa", "Citá solo rutas que existen.")] },
      other: { dev: [lesson("m-bbbb", "citá  solo rutas, que existen")] },
    },
  });
  try {
    const s = stub();
    const { entry } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(s) });
    const p = entry!.proposal!;
    assert.equal(p.target, "plugin/agents/dev.md");
    assert.deepEqual(p.shared_lesson, { text: "Citá solo rutas que existen.", products: ["demo", "other"] });
    assert.equal(p.content, GLOBAL_DEV.replace("You are the dev.", RULE));
    assert.ok("new_global_body" in (s.calls.find((c) => c.model === PROPOSAL_MODEL)!.schema as { properties: object }).properties);
    assert.equal(p.status, "pending");

    const res = await settleReflection("demo", entry!.id, "approved", { root: f.root });
    assert.equal(res.status, "approved");
    assert.equal(readFileSync(join(f.root, "plugin/agents/dev.md"), "utf8"), p.content);
    const files = (await git(f.root, "show", "--name-only", "--format=", "HEAD")).stdout.trim().split("\n").sort();
    assert.deepEqual(files, ["plugin/agents/dev.md"], "only the global role file is committed; the reflection is a database row");
  } finally {
    await f.cleanup();
  }
});

test("the baseline is re-run with today's prompt when the stored prompt hash differs", async () => {
  const f = await fixture({ devHash: "old" });
  try {
    const s = stub();
    const { entry } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(s) });
    const p = entry!.proposal!;
    assert.deepEqual(p.replay.rows.map((r) => r.baseline), ["rerun", "rerun"]);
    const roleCalls = s.calls.filter((c) => c.schema === roleAnswerSchema);
    assert.equal(roleCalls.filter((c) => !c.system.includes(RULE)).length, 2);
    assert.equal(roleCalls.filter((c) => c.system.includes(RULE)).length, 2);
  } finally {
    await f.cleanup();
  }
});

test("a replay over budget is discarded with the reason", async () => {
  const f = await fixture();
  try {
    const { entry } = await reflect(f.product, { root: f.root, now: NOW, warn: quiet, ...deps(stub()), budgetUsd: 0.4 });
    const p = entry!.proposal!;
    assert.equal(p.status, "discarded");
    assert.match(p.replay.aborted!, /budget of 0\.4 USD exhausted/);
    assert.equal(p.replay.totals, null);
    assert.ok(p.replay.cost_usd <= 0.4);
  } finally {
    await f.cleanup();
  }
});

const earlier = (over: Partial<ReflectionEntry["proposal"] & object>): ReflectionEntry => ({
  id: "r-old",
  product: "demo",
  created_at: "2026-09-01T00:00:00.000Z",
  trigger: "manual",
  signals: [],
  memory: [],
  proposal: {
    agent: "dev",
    target: "workspace/demo/agents/dev.md",
    trigger: { reason: "failure_class", failure_class: "unresolved_source", count: 3 },
    shared_lesson: null,
    rationale: "r",
    expected_effect: "e",
    diff: "",
    content: "",
    body_hash: sha256(normalizeLesson(`---\nextends: dev\n---\n${RULE}\n`)),
    target_hash: null,
    replay: { tuning: [], held_out: [], rows: [], totals: null, verdict: { passed: false, worse: [], better: [] }, cost_rule: "", cost_usd: 0, budget_usd: 15, aborted: null },
    decision_id: 3,
    status: "rejected",
    ...over,
  },
  status: "applied",
  commit: null,
});

test("reflect never re-proposes a change that was rejected before, even with other punctuation", async () => {
  const f = await fixture();
  try {
    const s = stub({ body: "cite only paths, you read" });
    const res = await proposeForReady(
      { product: f.product, root: f.root, reflectionId: "r-new", log: f.log, minutes: [], reflections: [earlier({})], items: async () => [], warn: quiet },
      deps(s),
    );
    assert.equal(res.proposal!.status, "discarded");
    assert.equal(res.proposal!.replay.aborted, "repite la propuesta de r-old (rejected)");
    assert.ok(!s.calls.some((c) => c.schema === roleAnswerSchema), "no replay for a repeated change");
  } finally {
    await f.cleanup();
  }
});

test("evidence counts from the agent's last proposal", async () => {
  const f = await fixture();
  try {
    const later = earlier({ status: "discarded" });
    later.created_at = "2026-10-05T00:00:00.000Z";
    const res = await proposeForReady(
      { product: f.product, root: f.root, reflectionId: "r-new", log: f.log, minutes: [], reflections: [later], items: async () => [], warn: quiet },
      deps(stub()),
    );
    assert.deepEqual(res, { candidates: [], proposal: null, cost_usd: 0 });
  } finally {
    await f.cleanup();
  }
});

const target = (over: Partial<ProposalTarget> = {}): ProposalTarget => ({
  path: "workspace/demo/agents/dev.md",
  global: false,
  role: "dev",
  current: null,
  editable: "",
  shared: null,
  ...over,
});

test("renderProposal keeps the frontmatter and lets the proposal change only model and effort", () => {
  assert.deepEqual(renderProposal(target(), "Do X."), { content: "---\nextends: dev\n---\nDo X.\n" });
  assert.deepEqual(renderProposal(target(), "---\nmodel: opus\neffort: high\n---\nDo X."), { content: "---\nextends: dev\nmodel: opus\neffort: high\n---\nDo X.\n" });
  assert.deepEqual(renderProposal(target(), "---\nextends: dev\n---\nDo X."), { content: "---\nextends: dev\n---\nDo X.\n" });
  assert.match((renderProposal(target(), "---\ntools: Read, Bash\n---\nDo X.") as { error: string }).error, /frontmatter "tools"/);
  assert.match((renderProposal(target(), "---\nextends: qa\n---\nDo X.") as { error: string }).error, /frontmatter "extends"/);
  assert.match((renderProposal(target(), "---\neffort: turbo\n---\nDo X.") as { error: string }).error, /effort inválido/);
  assert.match((renderProposal(target(), "x".repeat(4001)) as { error: string }).error, /4001 caracteres \(máximo 4000\)/);
  assert.match((renderProposal(target(), "  ") as { error: string }).error, /vacío/);

  const global = target({ path: "plugin/agents/dev.md", global: true, current: GLOBAL_DEV });
  assert.deepEqual(renderProposal(global, "---\nmodel: opus\n---\nNew body."), { content: GLOBAL_DEV.replace("model: sonnet", "model: opus").replace("You are the dev.", "New body.") });
  assert.ok("content" in renderProposal(global, "x".repeat(5000)), "the overlay cap does not apply to a global body");
});

test("chooseTarget picks the overlay unless the normalized lesson holds in two products", async () => {
  const f = await fixture({ memory: { other: { dev: [lesson("m-b", "Citá solo rutas que existen")] }, third: { qa: [lesson("m-c", "Otra cosa")] } } });
  try {
    const def = await loadRole("dev", "demo", roleDirsAt(f.root));
    assert.equal((await chooseTarget(f.root, "demo", def, [lesson("m-a", "Otra lección")])).path, "workspace/demo/agents/dev.md");
    assert.equal((await chooseTarget(f.root, "demo", def, [{ ...lesson("m-a", "CITÁ solo rutas que existen!"), kind: "fact" }])).global, false, "facts do not count");
    const t = await chooseTarget(f.root, "demo", def, [lesson("m-a", "CITÁ solo rutas   que existen!")]);
    assert.deepEqual([t.path, t.editable, t.shared?.products], ["plugin/agents/dev.md", "You are the dev.", ["demo", "other"]]);
  } finally {
    await f.cleanup();
  }
});

test("chooseTarget never picks a global role in a root without plugin/agents, such as a separate data root", async () => {
  const f = await fixture({ memory: { other: { dev: [lesson("m-b", "Citá solo rutas que existen")] } } });
  try {
    rmSync(join(f.root, "plugin"), { recursive: true, force: true });
    const def = await loadRole("dev", "demo", roleDirsAt(f.root));
    assert.equal(def.source, "global");
    const t = await chooseTarget(f.root, "demo", def, [lesson("m-a", "Citá solo rutas que existen")]);
    assert.deepEqual([t.path, t.global, t.shared], ["workspace/demo/agents/dev.md", false, null]);
  } finally {
    await f.cleanup();
  }
});

test("normalizeLesson lowercases, drops punctuation and collapses whitespace", () => {
  assert.equal(normalizeLesson("  Citá `solo` rutas,\n que EXISTEN. "), "citá solo rutas que existen");
});

test("the proposal system prompt names the owner, asks for the owner's language and keeps the no-data marker", () => {
  const system = proposalSystem(false, { name: "Ana", language: "Brazilian Portuguese" });
  assert.match(system, /the marks of Ana\./);
  assert.match(system, /Write both in Brazilian Portuguese: Ana reads them/);
  assert.doesNotMatch(system, /Rioplatense|Spanish/i);
  assert.match(system, /writing "\[no data\]" when data is missing/);
});
