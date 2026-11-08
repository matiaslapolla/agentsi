import { readBrief, type Brief } from "./brief.ts";
import { readMeetingLog, type MeetingLogEntry } from "./meeting-log.ts";
import { readMinutes } from "./minutes-store.ts";
import { MEMORY_MAX_CHARS, activeItems, memoryAgents, readMemoryItems, renderForPrompt, type MemoryItem } from "./memory.ts";
import { readReflections } from "./reflections.ts";
import type { Product } from "./registry.ts";
import { validateRoster } from "./roster.ts";
import type { Mark } from "./types.ts";

/** Meetings the audit looks at. */
export const AUDIT_WINDOW = 20;
/** Reflect proposes a prompt change when one agent shows the same failure class in this many meetings... */
export const READY_FAILURE_MEETINGS = 3;
/** ...or when this many of the priorities it owns were rejected. */
export const READY_REJECTED_PRIORITIES = 3;
/** `sin datos` in one meeting counts as a failure from this many occurrences. */
export const SIN_DATOS_FAILURE = 2;
/** An active item never marked helpful after this many meetings is reported. */
export const NEVER_HELPFUL_MEETINGS = 10;
/** The o1 signal compares repair rounds against this many first meetings. */
export const REPAIR_BASELINE_MEETINGS = 2;
/** The audit lists the repair rounds of this many last meetings, whatever their topic. */
export const REPAIR_LAST_MEETINGS = 4;

export type FailureClass = "unresolved_source" | "invented_source" | "missing_objective" | "timeout" | "sin_datos";

/** The synthesis is written by `pm`, so its failures are charged to that agent. */
export const SYNTHESIS_AGENT = "pm";

export interface MarkedMinutes {
  id: string;
  created_at: string;
  priorities: { owner_role: string; mark: Mark | null }[];
  check?: { repaired: boolean };
}

export interface ReflectionLine {
  id?: string;
  created_at?: string;
  status?: string;
  reverts?: string;
  consumed?: string[];
  proposal?: { status?: string; agent?: string } | null;
}

export interface AuditInput {
  product: Product;
  rosterFailures: string[];
  brief: Brief | null;
  log: MeetingLogEntry[];
  minutes: MarkedMinutes[];
  memory: Record<string, MemoryItem[]>;
  reflections: ReflectionLine[];
  now: Date;
}

export interface AgentMeetingStats {
  agent: string;
  meetings: number;
  answers: number;
  errors: number;
  timeouts: number;
  unresolved_sources_per_meeting: number;
  sin_datos_per_meeting: number;
  mean_cost_usd: number;
}

export interface MarkCounts {
  accepted: number;
  changed: number;
  rejected: number;
  unmarked: number;
}

export interface AgentMemoryStats {
  agent: string;
  active: number;
  retired: number;
  lessons: number;
  never_helpful: number;
  chars: number;
  max_chars: number;
}

export interface RepairGroup {
  minutes: string[];
  repaired: number;
  /** A meeting has at most one repair round, so this is also the repair rate. */
  per_meeting: number | null;
}

export interface RepairComparison {
  baseline: RepairGroup;
  since: RepairGroup;
}

export interface ReadyAgent {
  agent: string;
  reason: "failure_class" | "rejected_priorities";
  failure_class: FailureClass | null;
  count: number;
}

export interface AuditReport {
  product: string;
  generated_at: string;
  config: { valid: boolean; failures: string[]; overdue_objectives: { id: string; due: string; text: string }[] };
  meetings: {
    count: number;
    first_pass_rate: number | null;
    repair_rate: number | null;
    synthesis_cost_share: number | null;
    invented_sources: number;
    agents: AgentMeetingStats[];
  };
  repairs: RepairComparison;
  /** Repair rounds of the last meetings of the log, oldest first; null with fewer than REPAIR_LAST_MEETINGS meetings. */
  last_repairs: { minutes_id: string; rounds: number }[] | null;
  marks: Record<string, MarkCounts>;
  memory: AgentMemoryStats[];
  reflections: { last: { id: string; created_at: string } | null; pending_proposals: number; reverted: number };
  ready_to_reflect: ReadyAgent[];
}

const TIMEOUT = /timeout|timed out|etimedout/i;
const MISSING_OBJECTIVE = /\bobjective\b.*must be one of the brief ids/;

export const isMissingObjective = (failure: string): boolean => MISSING_OBJECTIVE.test(failure);

/** Failure classes each agent shows in one meeting. */
export function failureClasses(entry: MeetingLogEntry): Record<string, FailureClass[]> {
  const out: Record<string, FailureClass[]> = {};
  const add = (agent: string, c: FailureClass) => (out[agent] ??= []).push(c);
  for (const [agent, a] of Object.entries(entry.agents)) {
    if (a.unresolved_sources.length) add(agent, "unresolved_source");
    if (a.error && TIMEOUT.test(a.error)) add(agent, "timeout");
    if (a.sin_datos >= SIN_DATOS_FAILURE) add(agent, "sin_datos");
  }
  if (entry.synthesis.invented_sources.length) add(SYNTHESIS_AGENT, "invented_source");
  if (entry.synthesis.first_check.failures.some(isMissingObjective)) add(SYNTHESIS_AGENT, "missing_objective");
  return out;
}

/** Evidence that earlier reflections already processed. */
export interface Consumed {
  ids: Set<string>;
  /** Lines written before reflections recorded `consumed` count everything up to their time. */
  until: string | null;
}

export const NOTHING_CONSUMED: Consumed = { ids: new Set(), until: null };

/** What the given reflections processed: the minutes ids and memory sources in their `consumed`. Reverts consume nothing. */
export function consumedBy(lines: ReflectionLine[]): Consumed {
  const ids = new Set<string>();
  let until: string | null = null;
  for (const r of lines) {
    if (r.reverts || !r.created_at) continue;
    if (Array.isArray(r.consumed)) for (const id of r.consumed) ids.add(id);
    else if (until === null || r.created_at > until) until = r.created_at;
  }
  return { ids, until };
}

export const isConsumed = (c: Consumed, id: string, at: string): boolean => c.ids.has(id) || (c.until !== null && at <= c.until);

/** Agents whose evidence not yet consumed crosses a reflect threshold. */
export function readyToReflect(log: MeetingLogEntry[], minutes: MarkedMinutes[], consumed: Consumed = NOTHING_CONSUMED): ReadyAgent[] {
  const ready: ReadyAgent[] = [];

  const meetingsWith = new Map<string, Map<FailureClass, number>>();
  for (const entry of log.filter((e) => !isConsumed(consumed, e.minutes_id, e.created_at))) {
    for (const [agent, classes] of Object.entries(failureClasses(entry))) {
      const counts = meetingsWith.get(agent) ?? new Map<FailureClass, number>();
      for (const c of new Set(classes)) counts.set(c, (counts.get(c) ?? 0) + 1);
      meetingsWith.set(agent, counts);
    }
  }
  for (const [agent, counts] of meetingsWith) {
    for (const [failure_class, count] of counts) {
      if (count >= READY_FAILURE_MEETINGS) ready.push({ agent, reason: "failure_class", failure_class, count });
    }
  }

  const rejected = new Map<string, number>();
  for (const m of minutes.filter((m) => !isConsumed(consumed, m.id, m.created_at))) {
    for (const p of m.priorities) if (p.mark === "rejected") rejected.set(p.owner_role, (rejected.get(p.owner_role) ?? 0) + 1);
  }
  for (const [agent, count] of rejected) {
    if (count >= READY_REJECTED_PRIORITIES) ready.push({ agent, reason: "rejected_priorities", failure_class: null, count });
  }
  return ready.sort((a, b) => a.agent.localeCompare(b.agent) || b.count - a.count);
}

/**
 * Agents ready for a prompt proposal, strongest evidence first, one reason each. Each agent's evidence excludes what the
 * reflections up to its last proposal (whatever came of it) consumed, so an agent that was deferred or lacked replay
 * cases keeps its evidence, and a meeting that landed after that proposal counts even when it started before.
 */
export function readyForProposal(log: MeetingLogEntry[], minutes: MarkedMinutes[], reflections: ReflectionLine[]): ReadyAgent[] {
  const consumed = (agent: string): Consumed => {
    const last = reflections.findLastIndex((r) => r.proposal?.agent === agent && r.created_at);
    return last < 0 ? NOTHING_CONSUMED : consumedBy(reflections.slice(0, last + 1));
  };
  const agents = new Set(readyToReflect(log, minutes).map((r) => r.agent));
  return [...agents]
    .flatMap((agent) => readyToReflect(log, minutes, consumed(agent)).filter((r) => r.agent === agent).slice(0, 1))
    .sort((a, b) => b.count - a.count || a.agent.localeCompare(b.agent));
}

const rate = (n: number, of: number): number | null => (of ? n / of : null);
const sum = (xs: number[]): number => xs.reduce((a, b) => a + b, 0);

function agentStats(window: MeetingLogEntry[], agent: string): AgentMeetingStats {
  const runs = window.flatMap((e) => (e.agents[agent] ? [e.agents[agent]] : []));
  const n = runs.length;
  return {
    agent,
    meetings: n,
    answers: runs.filter((r) => r.error === null).length,
    errors: runs.filter((r) => r.error !== null).length,
    timeouts: runs.filter((r) => r.error !== null && TIMEOUT.test(r.error)).length,
    unresolved_sources_per_meeting: n ? sum(runs.map((r) => r.unresolved_sources.length)) / n : 0,
    sin_datos_per_meeting: n ? sum(runs.map((r) => r.sin_datos)) / n : 0,
    mean_cost_usd: n ? sum(runs.map((r) => r.cost_usd)) / n : 0,
  };
}

/** Repairs from the minutes, which exist for every meeting; the ledger misses the meetings before it existed. */
export function repairBaseline(minutes: MarkedMinutes[]): RepairComparison {
  type Checked = MarkedMinutes & Required<Pick<MarkedMinutes, "check">>;
  const checked = minutes.filter((m): m is Checked => m.check !== undefined).sort((a, b) => a.created_at.localeCompare(b.created_at));
  const group = (ms: Checked[]): RepairGroup => {
    const repaired = ms.filter((m) => m.check.repaired).length;
    return { minutes: ms.map((m) => m.id), repaired, per_meeting: rate(repaired, ms.length) };
  };
  return { baseline: group(checked.slice(0, REPAIR_BASELINE_MEETINGS)), since: group(checked.slice(REPAIR_BASELINE_MEETINGS)) };
}

function markCounts(minutes: MarkedMinutes[]): Record<string, MarkCounts> {
  const out: Record<string, MarkCounts> = {};
  for (const m of minutes) {
    for (const p of m.priorities) {
      const c = (out[p.owner_role] ??= { accepted: 0, changed: 0, rejected: 0, unmarked: 0 });
      c[p.mark ?? "unmarked"]++;
    }
  }
  return out;
}

function memoryStats(agent: string, items: MemoryItem[], log: MeetingLogEntry[]): AgentMemoryStats {
  const active = activeItems(items);
  const meetingsAfter = (at: string) => log.filter((e) => e.created_at > at).length;
  return {
    agent,
    active: active.length,
    retired: items.length - active.length,
    lessons: active.filter((i) => i.kind === "lesson").length,
    never_helpful: active.filter((i) => i.helpful === 0 && meetingsAfter(i.created_at) >= NEVER_HELPFUL_MEETINGS).length,
    chars: renderForPrompt(items, Infinity).length,
    max_chars: MEMORY_MAX_CHARS,
  };
}

/** The §8 report from already-read inputs; no I/O and no model call. */
export function buildAudit(input: AuditInput): AuditReport {
  const { product, log, minutes, reflections } = input;
  const window = log.slice(-AUDIT_WINDOW);
  const today = input.now.toISOString().slice(0, 10);
  const agents = [...new Set([...product.roles, ...window.flatMap((e) => Object.keys(e.agents))])];
  const total = sum(window.map((e) => e.total_cost_usd));

  const lines = reflections.filter((r) => r.created_at);
  const lastEntry = [...lines].reverse().find((r) => r.id && !r.reverts);
  const memoryAgents = [...new Set([...product.roles, ...Object.keys(input.memory)])].sort();

  return {
    product: product.id,
    generated_at: input.now.toISOString(),
    config: {
      valid: input.rosterFailures.length === 0,
      failures: input.rosterFailures,
      overdue_objectives: (input.brief?.objectives ?? []).filter((o) => o.due < today).map(({ id, due, text }) => ({ id, due, text })),
    },
    meetings: {
      count: window.length,
      first_pass_rate: rate(window.filter((e) => e.synthesis.first_check.passed).length, window.length),
      repair_rate: rate(window.filter((e) => e.synthesis.repaired).length, window.length),
      synthesis_cost_share: total ? sum(window.map((e) => e.synthesis.cost_usd)) / total : null,
      invented_sources: sum(window.map((e) => e.synthesis.invented_sources.length)),
      agents: agents.map((a) => agentStats(window, a)),
    },
    repairs: repairBaseline(minutes),
    last_repairs: log.length < REPAIR_LAST_MEETINGS ? null : log.slice(-REPAIR_LAST_MEETINGS).map((e) => ({ minutes_id: e.minutes_id, rounds: e.synthesis.repaired ? 1 : 0 })),
    marks: markCounts(minutes),
    memory: memoryAgents.map((a) => memoryStats(a, input.memory[a] ?? [], log)),
    reflections: {
      last: lastEntry ? { id: lastEntry.id!, created_at: lastEntry.created_at! } : null,
      pending_proposals: reflections.filter((r) => r.proposal?.status === "pending" && r.status !== "reverted").length,
      reverted: reflections.filter((r) => r.reverts).length,
    },
    ready_to_reflect: readyForProposal(log, minutes, reflections),
  };
}

async function readAllMemory(product: Product): Promise<Record<string, MemoryItem[]>> {
  const agents = new Set([...product.roles, ...(await memoryAgents(product.id))]);
  return Object.fromEntries(await Promise.all([...agents].map(async (a) => [a, await readMemoryItems(product.id, a)] as const)));
}

/**
 * Reads the product's data and reports its health. Deterministic: it never calls a model.
 * A database that cannot answer leaves no brief, overlay or memory to report, as for a product that has none:
 * the numbers from the minutes, the usage and the meeting log still come out.
 */
export async function audit(product: Product, now = new Date()): Promise<AuditReport> {
  let reachable = true;
  const brief = await readBrief(product.id).catch(() => {
    reachable = false;
    return null;
  });
  return buildAudit({
    product,
    rosterFailures: reachable ? await validateRoster(product) : [],
    brief,
    // With the database out of reach there is nothing to read: the report is empty, as the memory section is.
    log: reachable ? await readMeetingLog(product.id) : [],
    minutes: reachable ? await readMinutes(product.id) : [],
    memory: reachable ? await readAllMemory(product) : Object.fromEntries(product.roles.map((a) => [a, []])),
    reflections: reachable ? await readReflections(product.id) : [],
    now,
  });
}
