import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { ClaudeCall, ClaudeCaller } from "./claude.ts";
import { getPool } from "./db.ts";
import { testDb } from "./test-db.ts";
import { roleAnswerSchema } from "./meeting.ts";
import { buildLogEntry, parseMeetingLog, type MeetingLogEntry } from "./meeting-log.ts";
import { replayTable } from "./proposal.ts";
import type { Owner, Product } from "./registry.ts";
import type { RoleDef } from "./roles.ts";
import {
  Budget,
  BudgetExceeded,
  COST_RULE,
  REPLAY_BUDGET_USD,
  compareTotals,
  insufficientCases,
  isHeldOut,
  replay,
  replayCases,
  splitCases,
  storedMetrics,
  totals,
  type CaseMetrics,
  type ReplayCase,
  type ReplayInput,
  type ReplayReport,
  type ReplayTotals,
} from "./replay.ts";
import type { Minutes, UsageRow } from "./types.ts";

const metrics = (o: Partial<CaseMetrics> = {}): CaseMetrics => ({ first_pass: true, unresolved: 0, invented: 0, missing_objectives: 0, sin_datos: 0, cost_usd: 0.1, ...o });

function entry(id: string, agents: MeetingLogEntry["agents"]): MeetingLogEntry {
  return {
    minutes_id: id,
    created_at: "2026-10-01T00:00:00.000Z",
    question: "q",
    scope: null,
    brief_hash: null,
    agents,
    synthesis: {
      first_check: { passed: false, failures: ['Priority #0 "t" objective is missing and must be one of the brief ids: o1.', "x"] },
      repaired: true,
      final_check: { passed: true, failures: [] },
      invented_sources: [{ type: "path", ref: "a" }],
      cost_usd: 0,
    },
    total_cost_usd: 0,
  };
}

const run = (o: Partial<MeetingLogEntry["agents"][string]> = {}): MeetingLogEntry["agents"][string] => ({
  prompt_hash: "h",
  memory_hash: "",
  model: "sonnet",
  cost_usd: 0.02,
  duration_ms: 1,
  error: null,
  priorities: 1,
  sources: 1,
  unresolved_sources: [{ type: "path", ref: "x" }],
  sin_datos: 0,
  ...o,
});

test("the split by id hash is stable: a case keeps its half as cases accumulate", () => {
  const ids = Array.from({ length: 12 }, (_, i) => ({ minutes_id: `2026-10-${String(i + 1).padStart(2, "0")}-1200-q` }));
  const small = splitCases(ids.slice(0, 6));
  const large = splitCases(ids);
  for (const c of small.heldOut) assert.ok(large.heldOut.includes(c));
  for (const c of small.tuning) assert.ok(large.tuning.includes(c));
  assert.equal(large.heldOut.length + large.tuning.length, 12);
  assert.equal(isHeldOut("2026-10-02-1200-q"), true);
  assert.equal(isHeldOut("2026-10-01-1200-q"), false);
});

test("insufficientCases needs 4 cases, 2 held out and 1 for tuning", () => {
  const n = (t: number, h: number) => insufficientCases({ tuning: Array(t).fill(0), heldOut: Array(h).fill(0) });
  assert.match(n(2, 1)!, /3 casos de replay \(mínimo 4\)/);
  assert.match(n(3, 1)!, /1 casos held-out \(mínimo 2\)/);
  assert.match(n(0, 4)!, /ningún caso de tuning/);
  assert.equal(n(2, 2), null);
});

test("guards: nothing worse and one metric better; cost is worse only above +25% and never an improvement", () => {
  const base = totals([metrics({ first_pass: false, unresolved: 2 }), metrics()]);
  assert.deepEqual(compareTotals(base, totals([metrics({ unresolved: 1 }), metrics()])), {
    passed: true,
    worse: [],
    better: ["first_pass_rate", "unresolved_per_case"],
  });
  assert.deepEqual(compareTotals(base, totals([metrics({ unresolved: 1, invented: 1 }), metrics()])).worse, ["invented"]);
  assert.deepEqual(compareTotals(base, base), { passed: false, worse: [], better: [] });
  assert.equal(compareTotals(base, totals([metrics({ cost_usd: 0.01 }), metrics({ cost_usd: 0.01 })])).passed, true, "cheaper with a real gain passes");
  assert.deepEqual(compareTotals(base, totals([metrics({ first_pass: false, unresolved: 2, cost_usd: 0.01 }), metrics({ cost_usd: 0.01 })])).better, []);
  assert.deepEqual(compareTotals(base, totals([metrics({ cost_usd: 0.125 }), metrics({ cost_usd: 0.125 })])).worse, [], "+25% is tolerated");
  assert.deepEqual(compareTotals(base, totals([metrics({ cost_usd: 0.13 }), metrics({ cost_usd: 0.13 })])).worse, ["cost_usd"]);
  assert.deepEqual(compareTotals(totals([metrics({ cost_usd: 0 })]), totals([metrics({ cost_usd: 1 })])).worse, [], "no stored cost, no cost guard");
});

test("stored metrics come from the meeting log line", () => {
  assert.deepEqual(storedMetrics(entry("m", { dev: run() }), "dev"), { first_pass: false, unresolved: 1, invented: 1, missing_objectives: 1, sin_datos: 0, cost_usd: 0.02 });
});

test("replay cases need a snapshot, saved minutes and an answer from the agent", async () => {
  const db = await testDb();
  const root = mkdtempSync(join(tmpdir(), "agentsi-replay-"));
  try {
    const save = async (id: string, answer: string, snapshot = true) => {
      await getPool().query("insert into minutes (id, product, data) values ($1, 'demo', $2::jsonb)", [id, JSON.stringify({ id, roles: { dev: { answer } } })]);
      if (snapshot) await getPool().query("insert into snapshots (product, minutes_id, body) values ('demo', $1, '# Snapshot')", [id]);
    };
    await save("ok", "view");
    await save("empty", " ");
    await save("nosnap", "view", false);
    await save("failed", "view");
    const log = [entry("ok", { dev: run() }), entry("empty", { dev: run() }), entry("nosnap", { dev: run() }), entry("failed", { dev: run({ error: "boom" }) }), entry("absent", { qa: run() }), entry("nominutes", { dev: run() })];
    assert.deepEqual((await replayCases(root, "demo", "dev", log)).map((c) => [c.minutes_id, c.snapshot]), [["ok", "# Snapshot"]]);
  } finally {
    rmSync(root, { recursive: true, force: true });
    await db.close();
  }
});

test("the budget starts a call only when its cap fits, charges a failed call its cap and passes the cap down", async () => {
  const seen: ClaudeCall[] = [];
  const ok = async (c: ClaudeCall) => {
    seen.push(c);
    return { output: {}, usage: { model: "m", input_tokens: 0, output_tokens: 0, cache_read: 0, cache_write: 0, cost_usd: 0.4, duration_ms: 1 } };
  };
  const call = { prompt: "", model: "m", system: "", cwd: "/", tools: [] };
  const b = new Budget(2, 1);
  await b.run(ok, call);
  assert.equal(seen[0]!.maxBudgetUsd, 1);
  await assert.rejects(b.run(async () => Promise.reject(new Error("boom")), call), /boom/);
  assert.equal(b.spent, 1.4);
  await assert.rejects(b.run(ok, call), BudgetExceeded);
  assert.equal(seen.length, 1);
});

test("replay never imports the writers of minutes, memory or the database, and runs no SQL that writes", () => {
  const source = readFileSync(new URL("./replay.ts", import.meta.url), "utf8");
  assert.doesNotMatch(source, /writeMinutes|writeMemoryItems|appendFacts|writeLedger|storeMinutes|insert into|update \w+ set|delete from/i);
});

// T004: sin_datos ("no data" answers) is a case metric and a total of the replay gate.
type WithSin = CaseMetrics & { sin_datos: number };
type TotalsWithSin = ReplayTotals & { sin_datos: number };
const withSin = (sin_datos: number, o: Partial<CaseMetrics> = {}): WithSin => ({ ...metrics(o), sin_datos }) as WithSin;

test("T004: a rise in sin_datos with equal other metrics makes the replay verdict worse, and the verdict names sin_datos", () => {
  // The prompt now answers "no data" in one more case; nothing else moves.
  const before = totals([withSin(0), withSin(0)]);
  const after = totals([withSin(1), withSin(0)]);
  assert.deepEqual(compareTotals(before, after), { passed: false, worse: ["sin_datos"], better: [] });
});

test("T004: a fall in sin_datos is only a guard: the lower count reaches the totals and never counts as better", () => {
  const before = totals([withSin(2), withSin(0)]) as TotalsWithSin;
  const after = totals([withSin(1), withSin(0)]) as TotalsWithSin;
  assert.equal(after.sin_datos, 1, "the lower count is in the replay totals");
  // Equal everything else: the gate must not pass, and the fall is neither worse nor better.
  assert.deepEqual(compareTotals(before, after), { passed: false, worse: [], better: [] });
  // A real gain in first_pass with a fall in sin_datos: sin_datos is not among the reasons for the gain.
  const gainBefore = totals([withSin(2, { first_pass: false }), withSin(0, { first_pass: false })]);
  const gainAfter = totals([withSin(1), withSin(0)]);
  assert.deepEqual(compareTotals(gainBefore, gainAfter), { passed: true, worse: [], better: ["first_pass_rate"] });
});

test("T004: storedMetrics reads sin_datos from the meeting log into the case metrics and the totals", () => {
  assert.deepEqual(storedMetrics(entry("m", { dev: run({ sin_datos: 2 }) }), "dev"), {
    first_pass: false,
    unresolved: 1,
    invented: 1,
    missing_objectives: 1,
    cost_usd: 0.02,
    sin_datos: 2,
  });
  const cases = [
    storedMetrics(entry("a", { dev: run({ sin_datos: 2 }) }), "dev"),
    storedMetrics(entry("b", { dev: run({ sin_datos: 1 }) }), "dev"),
  ] as WithSin[];
  assert.equal((totals(cases) as TotalsWithSin).sin_datos, 3);
});

test("T004: the existing replay fixtures keep their verdicts with sin_datos carried, and read 0 where the log says 0", () => {
  // Fixture from the existing guard test: the log line has sin_datos 0.
  const stored = storedMetrics(entry("m", { dev: run() }), "dev") as WithSin;
  assert.equal(stored.sin_datos, 0);
  const base = totals([withSin(0, { first_pass: false, unresolved: 2 }), withSin(0)]);
  assert.deepEqual(compareTotals(base, totals([withSin(0, { unresolved: 1 }), withSin(0)])), {
    passed: true,
    worse: [],
    better: ["first_pass_rate", "unresolved_per_case"],
  });
  assert.deepEqual(compareTotals(base, totals([withSin(0, { cost_usd: 0.13 }), withSin(0, { cost_usd: 0.13 })])).worse, ["cost_usd"]);
});

// T005: the rerun and the stored baseline count sin_datos with one function, and an old log line without the field reads 0.
const T005_QUESTIONS = ["Cómo va el churn?", "Cuánto cuesta el hosting?", "Qué pasó con el onboarding?", "Cuántos usuarios activos hay?"];
const t005Owner: Owner = { name: "Ana", language: "Spanish" };
const t005Product = (dir: string): Product => ({ id: "demo", name: "Demo", paths: [dir], status: "active", roles: ["dev"] });
const dev: RoleDef = { id: "dev", role: "dev", source: "global", model: "sonnet", body: "dev prompt of today" };
const devCandidate: RoleDef = { ...dev, body: "dev prompt proposed" };
const pm: RoleDef = { id: "pm", role: "pm", source: "global", model: "sonnet", body: "pm prompt" };
const callUsage = (cost_usd = 0.02) => ({ model: "sonnet", input_tokens: 0, output_tokens: 0, cache_read: 0, cache_write: 0, cost_usd, duration_ms: 1 });

/** The model: each role answer repeats the text its question gets; the synthesis is empty. */
function model(answerFor: (question: string) => string): ClaudeCaller {
  return async (c) => {
    if (c.schema !== roleAnswerSchema) return { output: { priorities: [], disagreements: [], decisions: [], script: [] }, usage: callUsage() };
    const question = T005_QUESTIONS.find((q) => c.prompt.includes(q));
    if (!question) throw new Error("the role prompt does not carry the question");
    return { output: { answer: answerFor(question), priorities: [], concerns: [], memory_notes: [] }, usage: callUsage() };
  };
}

const passing = async () => ({ passed: true, failures: [] });
const deps = (call: ClaudeCaller) => ({ call, check: passing, resolve: async () => true });

/** A replay input over the frozen cases; each run gets its own budget. */
const t005Input = (heldOut: ReplayCase[]): ReplayInput => ({
  product: t005Product(tmpdir()),
  agent: "dev",
  candidate: devCandidate,
  current: dev,
  pm,
  memory: "",
  brief: null,
  owner: t005Owner,
  heldOut,
  budget: new Budget(15),
});

/** A meeting as the minutes file and the meeting log hold it. `prompt` decides whether replay reads the log as stored. */
function savedCase(id: string, question: string, answer: string, prompt: string, firstFailures: string[]): ReplayCase {
  const minutes: Minutes = {
    id,
    product: "demo",
    scope: null,
    question,
    created_at: "2026-10-01T00:00:00.000Z",
    roles: { dev: { model: "sonnet", answer, priorities: [], concerns: [], memory_notes: [] } },
    priorities: [],
    disagreements: [],
    decisions: [],
    script: [],
    check: { passed: firstFailures.length === 0, failures: [], first_failures: firstFailures, repaired: firstFailures.length > 0 },
    status: "valid",
  };
  const log = buildLogEntry({
    minutes,
    usage: [{ ...callUsage(), role: "dev" }],
    agents: { dev: { prompt, memory: "", model: "sonnet", error: null, duration_ms: 1, unresolved_sources: [] } },
    snapshot: "# Snapshot",
  });
  return { minutes_id: id, log, minutes, snapshot: "# Snapshot" };
}

test("T005: the rerun and the stored baseline count sin_datos the same way for '[no data]' and 'sin datos'", async () => {
  const churn = "[no data] for the churn of July.";
  const hosting = "Hosting: sin datos en la factura de agosto.";
  const answers: Record<string, string> = { [T005_QUESTIONS[0]!]: churn, [T005_QUESTIONS[1]!]: hosting };
  const answerFor = (q: string) => answers[q]!;
  const stored = [
    savedCase("2026-10-01-1200-churn", T005_QUESTIONS[0]!, churn, dev.body, []),
    savedCase("2026-10-02-1200-hosting", T005_QUESTIONS[1]!, hosting, dev.body, []),
  ];
  // Same meetings, but stored under an older prompt, so replay must re-run the agent with today's prompt.
  const rerun = [
    savedCase("2026-10-01-1200-churn", T005_QUESTIONS[0]!, churn, "an older dev prompt", []),
    savedCase("2026-10-02-1200-hosting", T005_QUESTIONS[1]!, hosting, "an older dev prompt", []),
  ];

  const fromStored = await replay(t005Input(stored), deps(model(answerFor)));
  const fromRerun = await replay(t005Input(rerun), deps(model(answerFor)));

  assert.equal(fromStored.aborted, null);
  assert.equal(fromRerun.aborted, null);
  assert.deepEqual(fromStored.rows.map((r) => r.baseline), ["stored", "stored"]);
  assert.deepEqual(fromRerun.rows.map((r) => r.baseline), ["rerun", "rerun"]);
  assert.deepEqual(fromStored.rows.map((r) => r.before.sin_datos), [1, 1], "the stored baseline counts both phrasings");
  assert.deepEqual(fromRerun.rows.map((r) => r.before.sin_datos), [1, 1], "the rerun counts both phrasings");
  assert.equal(fromRerun.totals!.before.sin_datos, fromStored.totals!.before.sin_datos, "the rerun and the stored baseline agree");
});

test("T005: an old log line without sin_datos reads as 0 in the stored baseline, and replay finishes", async () => {
  const db = await testDb();
  const root = mkdtempSync(join(tmpdir(), "agentsi-replay-t005-"));
  try {
    const saved = savedCase("2026-10-01-1200-churn", T005_QUESTIONS[0]!, "Churn subió 3% en julio.", dev.body, []);
    await getPool().query("insert into minutes (id, product, data) values ($1, 'demo', $2::jsonb)", [saved.minutes_id, JSON.stringify(saved.minutes)]);
    await getPool().query("insert into snapshots (product, minutes_id, body) values ('demo', $1, $2)", [saved.minutes_id, saved.snapshot]);
    // The line as the meeting log holds it when it was written before sin_datos existed.
    const legacy = JSON.parse(JSON.stringify(saved.log));
    delete legacy.agents.dev.sin_datos;
    const warnings: string[] = [];
    const log = parseMeetingLog(JSON.stringify(legacy) + "\n", (msg) => warnings.push(msg));
    assert.deepEqual(warnings, [], "the old line is read, not skipped");
    assert.equal(log.length, 1);

    const cases = await replayCases(root, "demo", "dev", log);
    assert.equal(cases.length, 1);
    const report = await replay({ ...t005Input(cases), product: t005Product(root) }, deps(model(() => "Churn bajó 1% en agosto.")));

    assert.equal(report.aborted, null);
    assert.equal(report.rows[0]!.baseline, "stored");
    assert.equal(report.rows[0]!.before.sin_datos, 0);
    assert.equal(report.totals!.before.sin_datos, 0);
  } finally {
    rmSync(root, { recursive: true, force: true });
    await db.close();
  }
});

test("T005: on a frozen 4-case fixture the gate discards a candidate that raises sin_datos and passes a clean one", async () => {
  const heldOut = T005_QUESTIONS.map((q, i) => savedCase(`2026-10-0${i + 1}-1200-replay`, q, "Churn subió 5% en julio.", dev.body, ["x"]));

  const clean = await replay(t005Input(heldOut), deps(model(() => "Churn bajó 2% en agosto.")));
  const raising = await replay(t005Input(heldOut), deps(model(() => "[no data] para el churn de agosto.")));

  assert.equal(clean.aborted, null);
  assert.equal(raising.aborted, null);
  assert.equal(clean.verdict.passed, true, "the clean candidate passes");
  assert.deepEqual(clean.verdict.worse, []);
  assert.equal(raising.totals!.after.sin_datos, 4);
  assert.equal(raising.verdict.passed, false, "the candidate that raises sin_datos is discarded");
  assert.ok(raising.verdict.worse.includes("sin_datos"), "the verdict names sin_datos");
});

// S006 criterion 1: the replay report prints the sin_datos count of each case, once for the baseline and once for the candidate.
test("S006: the replay report prints the sin_datos count of each case, for the baseline and for the candidate", async () => {
  const [churnQ, hostingQ] = [T005_QUESTIONS[0]!, T005_QUESTIONS[1]!];
  // The stored baseline: the churn answer carries two "no data" markers, the hosting answer none.
  const stored = [
    savedCase("2026-10-01-1200-churn", churnQ, "[no data] for the churn of July and sin datos for August.", dev.body, []),
    savedCase("2026-10-02-1200-hosting", hostingQ, "Hosting: 40 USD por mes.", dev.body, []),
  ];
  // The candidate: the churn answer carries none, the hosting answer one.
  const candidate: Record<string, string> = {
    [churnQ]: "Churn bajó 2% en agosto.",
    [hostingQ]: "[no data] para el hosting de agosto.",
  };
  const run = await replay(t005Input(stored), deps(model((q) => candidate[q]!)));
  assert.equal(run.aborted, null);
  assert.deepEqual(run.rows.map((r) => r.baseline), ["stored", "stored"]);

  const report: ReplayReport = {
    tuning: [],
    held_out: stored.map((c) => c.minutes_id),
    rows: run.rows,
    totals: run.totals,
    verdict: run.verdict,
    cost_rule: COST_RULE,
    cost_usd: 0,
    budget_usd: REPLAY_BUDGET_USD,
    aborted: run.aborted,
  };
  const lines = replayTable(report);

  // A case line reads "baseline → candidate": the baseline's count comes before the arrow, the candidate's after it.
  const caseSides = (minutesId: string): string[] => {
    const line = lines.find((l) => l.startsWith(`- ${minutesId}:`));
    assert.ok(line, `no case line for ${minutesId} in:\n${lines.join("\n")}`);
    return line.split(" → ");
  };
  const sinDatosIn = (side: string | undefined): number | null => {
    const n = side?.match(/sin datos\D*(\d+)/i)?.[1];
    return n === undefined ? null : Number(n);
  };

  const churn = caseSides(stored[0]!.minutes_id);
  assert.equal(churn.length, 2, `the case line has a baseline and a candidate side: ${churn.join(" | ")}`);
  assert.deepEqual([sinDatosIn(churn[0]), sinDatosIn(churn[1])], [2, 0], "churn: baseline 2, candidate 0");

  const hosting = caseSides(stored[1]!.minutes_id);
  assert.equal(hosting.length, 2, `the case line has a baseline and a candidate side: ${hosting.join(" | ")}`);
  assert.deepEqual([sinDatosIn(hosting[0]), sinDatosIn(hosting[1])], [0, 1], "hosting: baseline 0, candidate 1");
});
