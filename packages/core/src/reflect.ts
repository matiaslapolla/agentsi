import { SIN_DATOS_FAILURE, SYNTHESIS_AGENT, consumedBy, isConsumed, readyForProposal, type Consumed } from "./audit.ts";
import { readBrief, renderBriefXml } from "./brief.ts";
import { claude, type ClaudeCaller } from "./claude.ts";
import type { CommitResult } from "./exec.ts";
import { readMeetingLog, type MeetingLogEntry } from "./meeting-log.ts";
import { readMinutes } from "./minutes-store.ts";
import {
  activeItems,
  bumpCounter,
  makeItem,
  memoryAgents,
  normalizeText,
  readMemoryItems,
  renderForPrompt,
  retireItem,
  writeMemoryItems,
  type MemoryItem,
} from "./memory.ts";
import { withLock } from "./lock.ts";
import { dataRoot, workspaceLockPath } from "./paths.ts";
import { decisionText, proposeForReady, type ProposeDeps, type ProposeResult } from "./proposal.ts";
import { recordReflectionDecision } from "./reflection-decision.ts";
import type { Product } from "./registry.ts";
import {
  canRevert,
  readReflections,
  updateReflections,
  writeReflections,
  unifiedDiff,
  type MemoryChange,
  type Proposal,
  type ProposalCandidate,
  type ReflectionEntry,
  type Signal,
} from "./reflections.ts";
import type { Minutes } from "./types.ts";

export * from "./reflections.ts";

export const MAX_ADDS = 5;
export const MAX_RETIRES = 10;
export const CURATION_MODEL = "haiku";

export type MinutesEvidence = Pick<Minutes, "id" | "created_at" | "priorities" | "decisions">;

const signalKey = (s: Signal): string => `${s.agent}|${s.kind}|${s.ref}|${s.detail}`;
const refs = (sources: { ref: string }[]): string => sources.map((s) => s.ref).join(", ");

function logSignals(entry: MeetingLogEntry): Signal[] {
  const out: Signal[] = [];
  const ref = entry.minutes_id;
  for (const failure of entry.synthesis.first_check.failures) out.push({ agent: SYNTHESIS_AGENT, kind: "first_check_failure", ref, detail: failure });
  if (entry.synthesis.invented_sources.length) {
    out.push({ agent: SYNTHESIS_AGENT, kind: "invented_source", ref, detail: refs(entry.synthesis.invented_sources) });
  }
  for (const [agent, a] of Object.entries(entry.agents)) {
    if (a.error) out.push({ agent, kind: "error", ref, detail: a.error });
    if (a.unresolved_sources.length) out.push({ agent, kind: "unresolved_source", ref, detail: refs(a.unresolved_sources) });
    if (a.sin_datos >= SIN_DATOS_FAILURE) out.push({ agent, kind: "sin_datos", ref, detail: `${a.sin_datos} "sin datos"` });
  }
  return out;
}

function minutesSignals(m: MinutesEvidence): Signal[] {
  const out: Signal[] = [];
  for (const [idx, p] of m.priorities.entries()) {
    if (p.mark) out.push({ agent: p.owner_role, kind: `mark_${p.mark}`, ref: m.id, detail: `#${idx} ${p.title}` });
  }
  for (const d of m.decisions) {
    if (d.status === "rejected") out.push({ agent: SYNTHESIS_AGENT, kind: "decision_rejected", ref: m.id, detail: d.text });
  }
  return out;
}

/**
 * Grounded signals no earlier reflection consumed. Meeting-log lines count by minutes id, so a meeting that landed
 * after a reflection started counts in the next one. Marks, rejected decisions and reverts count when no earlier entry
 * already recorded them.
 */
export function collectSignals(i: { log: MeetingLogEntry[]; minutes: MinutesEvidence[]; reflections: ReflectionEntry[] }): Signal[] {
  const consumed = consumedBy(i.reflections);
  const seen = new Set(i.reflections.flatMap((e) => e.signals.map(signalKey)));
  const out: Signal[] = [];
  const push = (s: Signal) => {
    const key = signalKey(s);
    if (seen.has(key)) return;
    seen.add(key);
    out.push(s);
  };

  for (const entry of i.log.filter((e) => !isConsumed(consumed, e.minutes_id, e.created_at))) logSignals(entry).forEach(push);
  for (const m of i.minutes) minutesSignals(m).forEach(push);
  for (const revert of i.reflections.filter((e) => e.reverts)) {
    const reverted = i.reflections.find((e) => e.id === revert.reverts);
    for (const change of reverted?.memory ?? []) {
      push({ agent: change.agent, kind: "reverted_reflection", ref: revert.reverts!, detail: `reverted by ${revert.id}` });
    }
  }
  return out;
}

/** Item sources that are new to reflection: not consumed, and not a lesson a reflection wrote. */
const isNewItem = (consumed: Consumed, reflectionIds: Set<string>, i: MemoryItem): boolean =>
  !reflectionIds.has(i.source) && !isConsumed(consumed, i.source, i.created_at);

export interface Deltas {
  add: { text: string; evidence: string[] }[];
  retire: { id: string; why: string }[];
  helpful: string[];
  harmful: string[];
}

export const DELTA_SCHEMA = {
  type: "object",
  additionalProperties: false,
  required: ["add", "retire", "helpful", "harmful"],
  properties: {
    add: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["text", "kind", "evidence"],
        properties: {
          text: { type: "string" },
          kind: { type: "string", enum: ["fact", "lesson"] },
          evidence: { type: "array", items: { type: "string" } },
        },
      },
    },
    retire: {
      type: "array",
      items: {
        type: "object",
        additionalProperties: false,
        required: ["id", "why"],
        properties: { id: { type: "string" }, why: { type: "string" } },
      },
    },
    helpful: { type: "array", items: { type: "string" } },
    harmful: { type: "array", items: { type: "string" } },
  },
} as const;

const CURATION_SYSTEM = [
  "You curate the memory of one AgentSI agent. Memory is a list of items with ids; you never rewrite it.",
  "Return deltas only: `add` new items, `retire` items by id with a reason, mark ids `helpful` or `harmful`.",
  "Propose only deltas backed by the listed signals. Every add cites in `evidence` at least one signal ref exactly as listed.",
  "Learn from failures as well as successes: an accepted mark shows what to keep doing, a failure or a rejection shows what to avoid.",
  "Retire items that are stale, duplicated or contradicted by the signals. Status snapshots (pending work, dates of today) do not belong in memory.",
  `At most ${MAX_ADDS} adds and ${MAX_RETIRES} retires. Write new item texts in the language of the existing items, one short sentence each.`,
  "Empty arrays are a valid answer.",
].join("\n");

export function curationPrompt(o: { agent: string; items: MemoryItem[]; signals: Signal[]; briefXml: string | null }): string {
  const items = activeItems(o.items).map((i) => `[${i.id}] (${i.kind}, helpful ${i.helpful}, harmful ${i.harmful}) ${i.text}`);
  const signals = o.signals.map((s) => `- ref ${s.ref} · ${s.kind} · ${s.detail}`);
  return [
    ...(o.briefXml ? [o.briefXml, ""] : []),
    `<agent>${o.agent}</agent>`,
    "<items>",
    ...(items.length ? items : ["(no items)"]),
    "</items>",
    "<signals>",
    ...(signals.length ? signals : ["(no signals)"]),
    "</signals>",
  ].join("\n");
}

const strings = (x: unknown): string[] => (Array.isArray(x) ? x.filter((s): s is string => typeof s === "string") : []);
const records = (x: unknown): Record<string, unknown>[] =>
  Array.isArray(x) ? x.filter((r): r is Record<string, unknown> => typeof r === "object" && r !== null) : [];

/** Keeps only deltas on active ids and adds that cite the agent's signals, within the caps; the rest goes to `dropped`. */
export function validateDeltas(raw: unknown, items: MemoryItem[], signals: Signal[]): { deltas: Deltas; dropped: string[] } {
  const r = (typeof raw === "object" && raw !== null ? raw : {}) as Record<string, unknown>;
  const active = activeItems(items);
  const ids = new Set(active.map((i) => i.id));
  const known = new Set(signals.map((s) => s.ref));
  const texts = new Set(active.map((i) => normalizeText(i.text)));
  const deltas: Deltas = { add: [], retire: [], helpful: [], harmful: [] };
  const dropped: string[] = [];

  for (const a of records(r.add)) {
    const text = normalizeText(typeof a.text === "string" ? a.text : "");
    const evidence = [...new Set(strings(a.evidence).filter((e) => known.has(e)))];
    if (!text) dropped.push("add: empty text");
    else if (!evidence.length) dropped.push(`add "${text}": cites no signal`);
    else if (texts.has(text)) dropped.push(`add "${text}": duplicates an item`);
    else if (deltas.add.length >= MAX_ADDS) dropped.push(`add "${text}": over the limit of ${MAX_ADDS}`);
    else {
      texts.add(text);
      deltas.add.push({ text, evidence });
    }
  }

  for (const x of records(r.retire)) {
    const id = typeof x.id === "string" ? x.id : "";
    const why = typeof x.why === "string" ? normalizeText(x.why) : "";
    if (!ids.has(id)) dropped.push(`retire ${id || "(no id)"}: no active item`);
    else if (!why) dropped.push(`retire ${id}: no reason`);
    else if (deltas.retire.some((d) => d.id === id)) dropped.push(`retire ${id}: repeated`);
    else if (deltas.retire.length >= MAX_RETIRES) dropped.push(`retire ${id}: over the limit of ${MAX_RETIRES}`);
    else deltas.retire.push({ id, why });
  }

  for (const counter of ["helpful", "harmful"] as const) {
    for (const id of strings(r[counter])) {
      if (!ids.has(id)) dropped.push(`${counter} ${id}: no active item`);
      else if (!deltas[counter].includes(id)) deltas[counter].push(id);
    }
  }
  return { deltas, dropped };
}

interface Applied {
  items: MemoryItem[];
  added: string[];
  retired: string[];
  helpful: string[];
  harmful: string[];
  /** Deltas on items that changed since the curation call read them. */
  skipped: string[];
}

/** Every signal is a failure or a mark, so every add is a lesson. Ids that are no longer active are skipped. */
function applyDeltas(items: MemoryItem[], d: Deltas, source: string, now: Date): Applied {
  let out = [...items];
  const active = new Set(activeItems(items).map((i) => i.id));
  const texts = new Set(activeItems(items).map((i) => normalizeText(i.text)));
  const a: Applied = { items: [], added: [], retired: [], helpful: [], harmful: [], skipped: [] };
  for (const add of d.add) {
    if (texts.has(add.text)) {
      a.skipped.push(`add "${add.text}": duplicates an item`);
      continue;
    }
    texts.add(add.text);
    const item = makeItem(add.text, { kind: "lesson", source, created_at: now.toISOString(), taken: out.map((i) => i.id) });
    out.push(item);
    a.added.push(item.id);
  }
  for (const counter of ["helpful", "harmful"] as const) {
    for (const id of d[counter]) {
      if (!active.has(id)) a.skipped.push(`${counter} ${id}: no longer active`);
      else {
        out = bumpCounter(out, id, counter);
        a[counter].push(id);
      }
    }
  }
  for (const r of d.retire) {
    if (!active.has(r.id)) a.skipped.push(`retire ${r.id}: no longer active`);
    else {
      out = retireItem(out, r.id, r.why, now);
      a.retired.push(r.id);
    }
  }
  return { ...a, items: out };
}

const rendered = (items: MemoryItem[]): string => renderForPrompt(items, Infinity);

function newReflectionId(now: Date, taken: Set<string>): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  const base = `r-${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}-${pad(now.getHours())}${pad(now.getMinutes())}`;
  let id = base;
  for (let n = 2; taken.has(id); n++) id = `${base}-${n}`;
  return id;
}

async function readBriefXml(product: string): Promise<string | null> {
  try {
    const brief = await readBrief(product);
    return brief ? renderBriefXml(brief) : null;
  } catch {
    return null;
  }
}


export interface ReflectOptions extends Omit<ProposeDeps, "call"> {
  root?: string;
  now?: Date;
  call?: ClaudeCaller;
  warn?: (msg: string) => void;
  /** False curates memory only and makes no prompt proposal. */
  proposals?: boolean;
  recordDecision?: typeof recordReflectionDecision;
}

export interface ReflectResult {
  /** Null when no agent had signals or new items and no proposal was made, so nothing was written. */
  entry: ReflectionEntry | null;
  commit: CommitResult;
  /** Agents past the proposal threshold and what came of each; empty with `proposals: false`. */
  candidates: ProposalCandidate[];
}

/**
 * Curates each agent's memory from the signals since the last reflection, with one structured call per agent
 * that sees the items as lines and returns deltas. Then, unless `proposals` is false, proposes one prompt change
 * gated by replay. Writes one `reflections.jsonl` entry and one commit, after every call has returned.
 */
export async function reflect(product: Product, o: ReflectOptions = {}): Promise<ReflectResult> {
  const root = o.root ?? dataRoot;
  const now = o.now ?? new Date();
  const call = o.call ?? claude;
  const warn = o.warn ?? console.warn;
  const p = product.id;

  const reflections = await readReflections(p, root, warn);
  const log = await readMeetingLog(p, root, warn);
  const minutes = await readMinutes(p);
  const signals = collectSignals({ log, minutes, reflections });
  const consumed = consumedBy(reflections);
  const reflectionIds = new Set(reflections.map((e) => e.id));
  const newSources = new Map(
    await Promise.all(
      (await memoryAgents(p)).map(
        async (a) => [a, activeItems(await readMemoryItems(p, a)).filter((i) => isNewItem(consumed, reflectionIds, i)).map((i) => i.source)] as const,
      ),
    ),
  );
  const withNewItems = [...newSources].filter(([, sources]) => sources.length).map(([a]) => a);
  const consuming = [
    ...new Set([
      ...log.filter((e) => !isConsumed(consumed, e.minutes_id, e.created_at)).map((e) => e.minutes_id),
      ...minutes.filter((m) => !isConsumed(consumed, m.id, m.created_at)).map((m) => m.id),
      ...[...newSources.values()].flat(),
    ]),
  ].sort();
  const agents = [...new Set([...signals.map((s) => s.agent), ...withNewItems])].sort();
  const proposing = o.proposals !== false && readyForProposal(log, minutes, reflections).length > 0;
  if (!agents.length && !proposing) return { entry: null, commit: { committed: false }, candidates: [] };

  const briefXml = await readBriefXml(p);
  const curated = await Promise.all(
    agents.map(async (agent) => {
      const items = await readMemoryItems(p, agent);
      const own = signals.filter((s) => s.agent === agent);
      const res = await call({
        prompt: curationPrompt({ agent, items, signals: own, briefXml }),
        system: CURATION_SYSTEM,
        model: CURATION_MODEL,
        effort: "low",
        tools: [],
        schema: DELTA_SCHEMA,
        cwd: root,
      });
      return { agent, items, own, res };
    }),
  );

  const id = newReflectionId(now, new Set(reflections.map((e) => e.id)));
  const validated = curated.map(({ agent, items, own, res }) => ({ agent, items, ...validateDeltas(res.output, items, own) }));
  for (const { agent, dropped } of validated) for (const reason of dropped) warn(`reflect ${agent}: dropped ${reason}`);
  const curatedItems = new Map(validated.map((v) => [v.agent, applyDeltas(v.items, v.deltas, id, now).items]));

  const proposed: ProposeResult = proposing
    ? await proposeForReady(
        { product, root, reflectionId: id, log, minutes, reflections, items: async (a) => curatedItems.get(a) ?? (await readMemoryItems(p, a)), warn },
        { call, check: o.check, resolve: o.resolve, budgetUsd: o.budgetUsd },
      )
    : { candidates: [], proposal: null, cost_usd: 0 };
  if (!agents.length && !proposed.proposal) return { entry: null, commit: { committed: false }, candidates: proposed.candidates };

  // The calls above can take minutes; a meeting may have added facts meanwhile, so the deltas go on a fresh read.
  const written = await withLock(workspaceLockPath(p, root), async () => {
    const memory: MemoryChange[] = [];
    for (const { agent, deltas, dropped } of validated) {
      const before = await readMemoryItems(p, agent);
      const next = applyDeltas(before, deltas, id, now);
      for (const reason of next.skipped) warn(`reflect ${agent}: skipped ${reason}`);
      if (JSON.stringify(next.items) !== JSON.stringify(before)) await writeMemoryItems(p, agent, next.items);
      const diff = await unifiedDiff(rendered(before), rendered(next.items), `${p}/memory/${agent}`);
      memory.push({ agent, added: next.added, retired: next.retired, helpful: next.helpful, harmful: next.harmful, dropped: [...dropped, ...next.skipped], diff });
    }
    const entry: ReflectionEntry = {
      id,
      product: p,
      created_at: now.toISOString(),
      trigger: "manual",
      signals,
      memory,
      proposal: proposed.proposal,
      ...(proposed.candidates.length ? { candidates: proposed.candidates } : {}),
      consumed: consuming,
      status: "applied",
      commit: null,
      cost_usd: curated.reduce((n, c) => n + c.res.usage.cost_usd, 0) + proposed.cost_usd,
    };
    await writeReflections(p, [...(await readReflections(p, root)), entry], root);
    return { entry };
  });
  const entry = written.entry.proposal?.status === "pending" ? await attachDecision(written.entry, o.recordDecision ?? recordReflectionDecision, root, warn) : written.entry;
  return { entry, commit: { committed: false }, candidates: proposed.candidates };
}

/**
 * Records the pending proposal's decision once its entry is on disk, so no decision points to a missing reflection.
 * When the insert fails the proposal is discarded with the reason, and the curated memory is still kept.
 */
async function attachDecision(entry: ReflectionEntry, record: typeof recordReflectionDecision, root: string, warn: (msg: string) => void): Promise<ReflectionEntry> {
  const proposal = entry.proposal!;
  let next: Proposal;
  try {
    const { text, why } = decisionText(proposal, entry.id);
    next = { ...proposal, decision_id: await record(entry.product, entry.id, text, why) };
  } catch (err) {
    const reason = `no se pudo registrar la decisión: ${(err as Error).message}`;
    warn(`reflect ${proposal.agent}: ${reason}`);
    next = { ...proposal, status: "discarded", replay: { ...proposal.replay, aborted: reason } };
  }
  await updateReflections(entry.product, (entries) => entries.map((e) => (e.id === entry.id ? { ...e, proposal: next } : e)), root);
  return { ...entry, proposal: next };
}

/**
 * Applies the inverse of one entry's memory changes on today's items, so later counters survive: its additions are
 * retired and marked harmful, its retirements come back, its counter bumps are undone. Appends a revert line and commits.
 */
export async function revertReflection(product: Product, id: string, o: { root?: string; now?: Date } = {}): Promise<ReflectResult> {
  const root = o.root ?? dataRoot;
  const now = o.now ?? new Date();
  const p = product.id;
  const { entry } = await withLock(workspaceLockPath(p, root), () => revertLocked(p, id, root, now));
  return { entry, commit: { committed: false }, candidates: [] };
}

async function revertLocked(p: string, id: string, root: string, now: Date): Promise<{ entry: ReflectionEntry }> {
  const reflections = await readReflections(p, root);
  const target = reflections.find((e) => e.id === id);
  if (!target) throw new Error(`Reflection "${id}" not found in ${p}.`);
  if (target.reverts) throw new Error(`Reflection "${id}" is a revert; it cannot be reverted.`);
  if (target.status === "reverted" || reflections.some((e) => e.reverts === id)) throw new Error(`Reflection "${id}" is already reverted.`);
  if (!canRevert(target)) throw new Error(`Reflection "${id}" changed no memory; there is nothing to revert.`);

  const revertId = newReflectionId(now, new Set(reflections.map((e) => e.id)));
  const memory: MemoryChange[] = [];
  for (const change of target.memory) {
    const before = await readMemoryItems(p, change.agent);
    let items = before;
    const exists = (itemId: string) => items.some((i) => i.id === itemId);
    const retired: string[] = [];
    const harmful: string[] = [];
    const restored: string[] = [];
    for (const itemId of change.added.filter(exists)) {
      if (items.find((i) => i.id === itemId)!.retired_at === null) {
        items = retireItem(items, itemId, `reverted by ${revertId}`, now);
        retired.push(itemId);
      }
      items = bumpCounter(items, itemId, "harmful");
      harmful.push(itemId);
    }
    for (const itemId of change.retired.filter(exists)) {
      if (items.find((i) => i.id === itemId)!.retired_at !== target.created_at) continue;
      items = items.map((i) => (i.id === itemId ? { ...i, retired_at: null, retired_why: null } : i));
      restored.push(itemId);
    }
    for (const counter of ["helpful", "harmful"] as const) {
      for (const itemId of change[counter].filter(exists)) {
        items = items.map((i) => (i.id === itemId ? { ...i, [counter]: Math.max(0, i[counter] - 1) } : i));
      }
    }
    if (JSON.stringify(items) === JSON.stringify(before)) continue;
    await writeMemoryItems(p, change.agent, items);
    const diff = await unifiedDiff(rendered(before), rendered(items), `${p}/memory/${change.agent}`);
    memory.push({ agent: change.agent, added: [], retired, helpful: [], harmful, restored, dropped: [], diff });
  }

  const entry: ReflectionEntry = {
    id: revertId,
    product: p,
    created_at: now.toISOString(),
    trigger: "manual",
    reverts: id,
    signals: [],
    memory,
    proposal: null,
    status: "applied",
    commit: null,
  };
  await writeReflections(p, [...reflections.map((e) => (e.id === id ? { ...e, status: "reverted" as const } : e)), entry], root);
  return { entry };
}
