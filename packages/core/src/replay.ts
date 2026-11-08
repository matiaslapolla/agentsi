import { SYNTHESIS_AGENT, isMissingObjective } from "./audit.ts";
import type { Brief } from "./brief.ts";
import type { CheckOptions, CheckResult } from "./check.ts";
import { defaultBudgetUsd, type ClaudeCall, type ClaudeCaller } from "./claude.ts";
import { buildMinutes, compactSnapshot, emptyAnswer, productCwd, roleCall, synthesisCall, toRoleAnswer, type Synthesis } from "./meeting.ts";
import { inventedSources, sha256, sinDatos, type MeetingLogEntry } from "./meeting-log.ts";
import { getPool } from "./db.ts";
import { readMinutes } from "./minutes-store.ts";
import type { Owner, Product } from "./registry.ts";
import type { RoleDef } from "./roles.ts";
import type { Minutes, RoleAnswer, Source } from "./types.ts";

export const REPLAY_MIN_CASES = 4;
/** The held-out half needs this many cases to compare anything. */
export const REPLAY_MIN_HELD_OUT = 2;
export const REPLAY_BUDGET_USD = 15;
/** The agent's cost counts as worse only when it rises more than this share. */
export const COST_TOLERANCE = 0.25;
export const COST_RULE = `cost counts as worse only above +${COST_TOLERANCE * 100}%; a lower cost is not an improvement (it is a budget constraint, not a goal)`;

export interface ReplayCase {
  minutes_id: string;
  log: MeetingLogEntry;
  minutes: Minutes;
  snapshot: string;
}

/** Meetings with a frozen snapshot and saved minutes in which the agent answered. Reads the database; the data root argument is ignored. */
export async function replayCases(_root: string, product: string, agent: string, log: MeetingLogEntry[]): Promise<ReplayCase[]> {
  const out: ReplayCase[] = [];
  const snapshots = new Map((await getPool().query<{ minutes_id: string; body: string }>("select minutes_id, body from snapshots where product = $1", [product])).rows.map((r) => [r.minutes_id, r.body]));
  const stored = new Map((await readMinutes(product)).map((m) => [m.id, m]));
  for (const entry of log) {
    const run = entry.agents[agent];
    const minutes = stored.get(entry.minutes_id);
    if (!run || run.error !== null || !minutes) continue;
    // A product with stored snapshots must have this one; one with none (minutes, usage and log only) replays with an empty snapshot.
    const snapshot = snapshots.get(entry.minutes_id);
    if (snapshot === undefined && snapshots.size) continue;
    if (!minutes.roles?.[agent]?.answer?.trim()) continue;
    out.push({ minutes_id: entry.minutes_id, log: entry, minutes, snapshot: snapshot ?? "" });
  }
  return out;
}

/** Held out when the first byte of sha256(minutes id) is even: a case never changes half as cases accumulate. */
export const isHeldOut = (minutesId: string): boolean => parseInt(sha256(minutesId).slice(0, 2), 16) % 2 === 0;

export function splitCases<T extends { minutes_id: string }>(cases: T[]): { tuning: T[]; heldOut: T[] } {
  return { tuning: cases.filter((c) => !isHeldOut(c.minutes_id)), heldOut: cases.filter((c) => isHeldOut(c.minutes_id)) };
}

/** Why the cases cannot back a proposal, or null when they can. */
export function insufficientCases(split: { tuning: unknown[]; heldOut: unknown[] }): string | null {
  const n = split.tuning.length + split.heldOut.length;
  if (n < REPLAY_MIN_CASES) return `${n} casos de replay (mínimo ${REPLAY_MIN_CASES})`;
  if (split.heldOut.length < REPLAY_MIN_HELD_OUT) return `${split.heldOut.length} casos held-out (mínimo ${REPLAY_MIN_HELD_OUT})`;
  if (!split.tuning.length) return "ningún caso de tuning para mostrarle a la propuesta";
  return null;
}

export interface CaseMetrics {
  first_pass: boolean;
  unresolved: number;
  invented: number;
  missing_objectives: number;
  /** "No data" answers; a stored line without the field reads 0. */
  sin_datos: number;
  cost_usd: number;
}

export interface ReplayTotals {
  cases: number;
  first_pass_rate: number;
  unresolved_per_case: number;
  invented: number;
  missing_objectives: number;
  sin_datos: number;
  cost_usd: number;
}

export interface ReplayRow {
  minutes_id: string;
  /** `stored`: the meeting log; `rerun`: re-run with today's prompt because the stored prompt hash differs. */
  baseline: "stored" | "rerun";
  before: CaseMetrics;
  after: CaseMetrics;
}

export interface ReplayVerdict {
  passed: boolean;
  worse: string[];
  better: string[];
}

export interface ReplayReport {
  tuning: string[];
  held_out: string[];
  rows: ReplayRow[];
  totals: { before: ReplayTotals; after: ReplayTotals } | null;
  verdict: ReplayVerdict;
  cost_rule: string;
  /** Spent by the proposal call and the replay calls. */
  cost_usd: number;
  budget_usd: number;
  /** Why the proposal was discarded before a full comparison (budget, invalid, repeated); null otherwise. */
  aborted: string | null;
}

export function storedMetrics(entry: MeetingLogEntry, agent: string): CaseMetrics {
  const run = entry.agents[agent]!;
  return {
    first_pass: entry.synthesis.first_check.passed,
    unresolved: run.unresolved_sources.length,
    invented: entry.synthesis.invented_sources.length,
    missing_objectives: entry.synthesis.first_check.failures.filter(isMissingObjective).length,
    sin_datos: run.sin_datos ?? 0,
    cost_usd: run.cost_usd,
  };
}

const sum = (xs: number[]): number => xs.reduce((a, b) => a + b, 0);

export function totals(ms: CaseMetrics[]): ReplayTotals {
  const n = ms.length;
  return {
    cases: n,
    first_pass_rate: n ? ms.filter((m) => m.first_pass).length / n : 0,
    unresolved_per_case: n ? sum(ms.map((m) => m.unresolved)) / n : 0,
    invented: sum(ms.map((m) => m.invented)),
    missing_objectives: sum(ms.map((m) => m.missing_objectives)),
    sin_datos: sum(ms.map((m) => m.sin_datos)),
    cost_usd: sum(ms.map((m) => m.cost_usd)),
  };
}

/** Passes when no metric is worse on the held-out half and at least one is better. */
export function compareTotals(before: ReplayTotals, after: ReplayTotals): ReplayVerdict {
  const worse: string[] = [];
  const better: string[] = [];
  if (after.first_pass_rate < before.first_pass_rate) worse.push("first_pass_rate");
  if (after.first_pass_rate > before.first_pass_rate) better.push("first_pass_rate");
  for (const k of ["unresolved_per_case", "invented", "missing_objectives"] as const) {
    if (after[k] > before[k]) worse.push(k);
    if (after[k] < before[k]) better.push(k);
  }
  // Guard only: a fall in sin_datos is never counted as better.
  if (after.sin_datos > before.sin_datos) worse.push("sin_datos");
  if (before.cost_usd > 0 && after.cost_usd > before.cost_usd * (1 + COST_TOLERANCE)) worse.push("cost_usd");
  return { passed: worse.length === 0 && better.length > 0, worse, better };
}

export class BudgetExceeded extends Error {}

/**
 * Spending cap over several calls. A call starts only when its own cap fits in what is left, so the total never
 * exceeds the limit. A failed call is charged its whole cap, since its real cost is unknown.
 */
export class Budget {
  spent = 0;
  private reserved = 0;
  readonly limit: number;
  private readonly fixedPerCall: number | undefined;

  /** Without `perCall`, each call is capped like a meeting call of its model. */
  constructor(limit: number, perCall?: number) {
    this.limit = limit;
    this.fixedPerCall = perCall;
  }

  async run(call: ClaudeCaller, c: ClaudeCall): ReturnType<ClaudeCaller> {
    const perCall = this.fixedPerCall ?? defaultBudgetUsd(c.model);
    if (this.spent + this.reserved + perCall > this.limit) {
      throw new BudgetExceeded(`replay budget of ${this.limit} USD exhausted (${this.spent.toFixed(2)} spent)`);
    }
    this.reserved += perCall;
    try {
      const res = await call({ ...c, maxBudgetUsd: perCall });
      this.spent += res.usage.cost_usd;
      return res;
    } catch (err) {
      if (!(err instanceof BudgetExceeded)) this.spent += perCall;
      throw err;
    } finally {
      this.reserved -= perCall;
    }
  }
}

export interface ReplayDeps {
  call: ClaudeCaller;
  check: (m: Minutes, opts: CheckOptions) => Promise<CheckResult>;
  resolve: (s: Source) => Promise<boolean>;
}

export interface ReplayInput {
  product: Product;
  agent: string;
  /** The agent with the proposed prompt. */
  candidate: RoleDef;
  /** The agent with today's prompt. */
  current: RoleDef;
  /** Today's pm, which writes the synthesis unless the agent is the pm. */
  pm: RoleDef;
  /** The agent's memory as its prompt gets it. */
  memory: string;
  brief: Brief | null;
  owner: Owner;
  heldOut: ReplayCase[];
  budget: Budget;
}

/** Re-runs one agent on a frozen meeting, keeps the other answers, re-runs synthesis and `check()`. Writes nothing. */
async function runCase(def: RoleDef, c: ReplayCase, i: ReplayInput, d: ReplayDeps): Promise<CaseMetrics> {
  const scope = c.log.scope;
  const cwd = productCwd(i.product, scope);
  const call = (x: ClaudeCall) => i.budget.run(d.call, x);

  let answer: RoleAnswer;
  let cost = 0;
  try {
    const r = await call(roleCall(def, { question: c.minutes.question, snap: c.snapshot, memory: i.memory, scope, cwd, brief: i.brief, owner: i.owner }, true));
    answer = toRoleAnswer(r.output, r.usage.model);
    cost = r.usage.cost_usd;
  } catch (err) {
    if (err instanceof BudgetExceeded) throw err;
    answer = emptyAnswer(def.model, (err as Error).message);
  }

  let unresolved = 0;
  for (const s of answer.priorities.flatMap((p) => p.sources)) if (!(await d.resolve(s))) unresolved++;

  const roles = { ...c.minutes.roles, [i.agent]: answer };
  const pm = i.agent === SYNTHESIS_AGENT ? def : i.pm;
  let synthesis: Synthesis | null = null;
  try {
    synthesis = (await call(synthesisCall(pm, c.minutes.question, roles, compactSnapshot(c.snapshot), cwd, i.brief, i.owner))).output as Synthesis;
  } catch (err) {
    if (err instanceof BudgetExceeded) throw err;
  }

  const { id, product, question, created_at } = c.minutes;
  const m = buildMinutes({ id, product, scope, question, created_at }, roles, synthesis, false);
  const res = await d.check(m, { roles: Object.keys(c.log.agents), product: i.product, brief: i.brief });
  return {
    first_pass: res.passed,
    unresolved,
    invented: inventedSources(m, c.snapshot).length,
    missing_objectives: res.failures.filter(isMissingObjective).length,
    sin_datos: sinDatos([answer.answer ?? "", ...(answer.concerns ?? [])].join("\n")),
    cost_usd: cost,
  };
}

export type ReplayRun = Pick<ReplayReport, "rows" | "totals" | "verdict" | "aborted">;

/**
 * Compares the proposed prompt against the baseline on the held-out cases. The baseline is the stored meeting,
 * re-run with today's prompt only when the stored prompt hash differs from it.
 */
export async function replay(i: ReplayInput, d: ReplayDeps): Promise<ReplayRun> {
  const today = sha256(i.current.body);
  const settled = await Promise.allSettled(
    i.heldOut.map(async (c): Promise<ReplayRow> => {
      const stored = c.log.agents[i.agent]!.prompt_hash === today;
      const before = stored ? storedMetrics(c.log, i.agent) : await runCase(i.current, c, i, d);
      const after = await runCase(i.candidate, c, i, d);
      return { minutes_id: c.minutes_id, baseline: stored ? "stored" : "rerun", before, after };
    }),
  );
  const failed = settled.find((s): s is PromiseRejectedResult => s.status === "rejected");
  if (failed) {
    if (!(failed.reason instanceof BudgetExceeded)) throw failed.reason;
    return { rows: [], totals: null, verdict: { passed: false, worse: [], better: [] }, aborted: (failed.reason as Error).message };
  }
  const rows = settled.map((s) => (s as PromiseFulfilledResult<ReplayRow>).value);
  const before = totals(rows.map((r) => r.before));
  const after = totals(rows.map((r) => r.after));
  return { rows, totals: { before, after }, verdict: compareTotals(before, after), aborted: null };
}
