import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join } from "node:path";
import { SYNTHESIS_AGENT, readyForProposal, type MarkedMinutes, type ReadyAgent } from "./audit.ts";
import { readBrief, renderBriefXml, type Brief } from "./brief.ts";
import { check, resolveSource, sourceContext, type CheckOptions, type CheckResult } from "./check.ts";
import type { ClaudeCaller } from "./claude.ts";
import { NO_DATA, sha256, type MeetingLogEntry } from "./meeting-log.ts";
import { MEMORY_MAX_CHARS, activeItems, allMemoryAgents, readMemoryItems, renderForPrompt, type MemoryItem } from "./memory.ts";
import { readOverlay } from "./overlays.ts";
import { loadOwner, type Owner, type Product } from "./registry.ts";
import { targetHash } from "./reflection-decision.ts";
import { unifiedDiff, type Proposal, type ProposalCandidate, type ReflectionEntry } from "./reflections.ts";
import {
  Budget,
  COST_RULE,
  REPLAY_BUDGET_USD,
  insufficientCases,
  replay,
  replayCases,
  splitCases,
  type CaseMetrics,
  type ReplayCase,
  type ReplayReport,
  type ReplayRun,
  type ReplayTotals,
} from "./replay.ts";
import { agentRole, loadRole, parseAgentFile, roleDirsAt, type RoleDef } from "./roles.ts";
import type { Source } from "./types.ts";

export const PROPOSAL_MODEL = "sonnet";
export const OVERLAY_MAX_CHARS = 4000;
/** The only frontmatter keys a proposal may change. */
export const EDITABLE_FRONTMATTER = ["model", "effort"];
const EFFORTS = ["low", "medium", "high", "xhigh", "max"];

/** Lowercase, no punctuation or symbols, collapsed whitespace: two lessons match only when this is equal. */
export const normalizeLesson = (s: string): string =>
  s
    .toLowerCase()
    .replace(/[\p{P}\p{S}]/gu, "")
    .replace(/\s+/g, " ")
    .trim();

/** A lesson of this agent that agents of the same role hold in other products, with the most products first. */
async function sharedLesson(root: string, product: string, role: string, items: MemoryItem[]): Promise<{ text: string; products: string[] } | null> {
  const own = new Map(activeItems(items).filter((i) => i.kind === "lesson").map((i) => [normalizeLesson(i.text), i.text]));
  if (!own.size) return null;
  const holders = new Map<string, Set<string>>();
  const dirs = roleDirsAt(root);
  for (const { product: other, agent } of await allMemoryAgents()) {
    if (other === product) continue;
    if ((await agentRole(agent, other, dirs)) !== role) continue;
    for (const i of activeItems(await readMemoryItems(other, agent))) {
      const key = normalizeLesson(i.text);
      if (i.kind === "lesson" && own.has(key)) holders.set(key, (holders.get(key) ?? new Set()).add(other));
    }
  }
  const best = [...holders].sort((a, b) => b[1].size - a[1].size || a[0].localeCompare(b[0]))[0];
  return best ? { text: own.get(best[0])!, products: [product, ...[...best[1]].sort()] } : null;
}

export interface ProposalTarget {
  /** Repo-relative path. */
  path: string;
  global: boolean;
  role: string;
  /** The file's text today; null when it does not exist yet. */
  current: string | null;
  /** The body the proposal rewrites: the overlay body, or the global role's body. */
  editable: string;
  shared: { text: string; products: string[] } | null;
}

/** The product overlay (created with `extends` when missing), or the global role when a lesson holds in 2+ products. */
export async function chooseTarget(root: string, product: string, def: RoleDef, items: MemoryItem[]): Promise<ProposalTarget> {
  const shared = def.source === "custom" ? null : await sharedLesson(root, product, def.role, items);
  // Only a root that holds the global roles may change them: a separate data root never writes into the code checkout.
  const global = shared !== null && existsSync(join(root, "plugin", "agents", `${def.role}.md`));
  // The overlay target is a name, not a file: the overlay lives in the database.
  const path = global ? `plugin/agents/${def.role}.md` : `workspace/${product}/agents/${def.id}.md`;
  const file = join(root, path);
  const current = global ? (existsSync(file) ? readFileSync(file, "utf8") : null) : await readOverlay(product, def.id);
  return { path, global, role: def.role, current, editable: current ? parseAgentFile(current).body : "", shared: global ? shared : null };
}

const FRONTMATTER = /^---\r?\n([\s\S]*?)\r?\n---\r?\n(\n?)/;

/** The target's new text from the proposed body; a leading frontmatter block may change only model and effort. */
export function renderProposal(t: ProposalTarget, proposed: string): { content: string } | { error: string } {
  const { frontmatter, body } = parseAgentFile(proposed);
  const currentFm = t.current ? parseAgentFile(t.current).frontmatter : { extends: t.role };
  const changes: Record<string, string> = {};
  for (const [key, value] of Object.entries(frontmatter)) {
    if (currentFm[key] === value) continue;
    if (!EDITABLE_FRONTMATTER.includes(key)) return { error: `cambia el frontmatter "${key}"; solo puede cambiar ${EDITABLE_FRONTMATTER.join(" y ")}` };
    if (key === "model" && !/^[a-z0-9][\w.-]*$/i.test(value)) return { error: `modelo inválido "${value}"` };
    if (key === "effort" && !EFFORTS.includes(value)) return { error: `effort inválido "${value}"` };
    changes[key] = value;
  }
  if (!body) return { error: "el cuerpo está vacío" };
  if (!t.global && body.length > OVERLAY_MAX_CHARS) return { error: `el overlay tiene ${body.length} caracteres (máximo ${OVERLAY_MAX_CHARS})` };

  const m = t.current ? FRONTMATTER.exec(t.current) : null;
  const lines = m ? m[1]!.split(/\r?\n/) : t.current ? [] : [`extends: ${t.role}`];
  const out = lines.map((line) => {
    const key = /^([A-Za-z][\w-]*):/.exec(line)?.[1];
    return key && changes[key] !== undefined ? `${key}: ${changes[key]}` : line;
  });
  for (const [key, value] of Object.entries(changes)) if (!lines.some((l) => l.startsWith(`${key}:`))) out.push(`${key}: ${value}`);
  const head = out.length ? `---\n${out.join("\n")}\n---\n${m?.[2] ?? ""}` : "";
  return { content: `${head}${body}\n` };
}

/** The agent as it would load with `content` in its target, without writing anything. */
async function effectiveWith(root: string, product: string, agent: string, t: ProposalTarget, content: string): Promise<RoleDef> {
  const base = roleDirsAt(root);
  if (!t.global) return loadRole(agent, product, { plugin: base.plugin, overlay: async (p, id) => (p === product && id === agent ? content : base.overlay(p, id)) });
  const tmp = mkdtempSync(join(tmpdir(), "agentsi-proposal-"));
  try {
    writeFileSync(join(tmp, basename(t.path)), content);
    return await loadRole(agent, product, { plugin: tmp, overlay: base.overlay });
  } finally {
    rmSync(tmp, { recursive: true, force: true });
  }
}

const bodyKey = (global: boolean) => (global ? "new_global_body" : "new_overlay_body");

export const proposalSchema = (global: boolean) => ({
  type: "object",
  additionalProperties: false,
  required: [bodyKey(global), "rationale", "expected_effect"],
  properties: {
    [bodyKey(global)]: { type: "string" },
    rationale: { type: "string" },
    expected_effect: { type: "string" },
  },
});

export function proposalSystem(global: boolean, owner: Owner): string {
  return [
    "You improve the prompt of one AgentSI agent from the traces of its past meetings.",
    `Read the traces: the agent's answers, the check failures, the sources that did not resolve, the invented sources and the marks of ${owner.name}. Find the one cause behind the repeated failure and change the instructions to remove it.`,
    "Change one thing. Keep the role's voice, its language and every rule that is not the cause.",
    `The check is a floor, not a goal. Never weaken a rule about sources or about writing "${NO_DATA}" when data is missing. Never tell the agent to cite a file it did not read, or a doc only to pass a check.`,
    global
      ? `Return in \`${bodyKey(true)}\` the complete new body of the global role file, without its frontmatter. Every product with this role gets it.`
      : `Return in \`${bodyKey(false)}\` the complete new body of the product overlay: the text appended to the global role under "## This product". At most ${OVERLAY_MAX_CHARS} characters.`,
    `You may start it with a frontmatter block that changes only ${EDITABLE_FRONTMATTER.join(" or ")}.`,
    `\`rationale\`: the cause you found, citing minutes ids. \`expected_effect\`: which replay metric should improve (first-pass check, unresolved sources, invented sources, missing objectives) and why. Write both in ${owner.language}: ${owner.name} reads them before approving.`,
  ].join("\n");
}

const srcs = (s: Source[]): string => (s.length ? s.map((x) => `${x.type} ${x.ref}`).join(", ") : "(none)");
const list = (xs: string[]): string => (xs.length ? xs.map((x) => `- ${x}`).join("\n") : "(none)");

export function proposalPrompt(o: {
  briefXml: string | null;
  agent: string;
  target: ProposalTarget;
  trigger: ReadyAgent;
  effective: string;
  tuning: ReplayCase[];
  history: { id: string; status: string; rationale: string }[];
}): string {
  const trigger = o.trigger.failure_class ? `${o.trigger.failure_class} in ${o.trigger.count} meetings` : `${o.trigger.count} rejected priorities`;
  const cases = o.tuning.map((c) => {
    const a = c.minutes.roles[o.agent]!;
    const marks = c.minutes.priorities.flatMap((p, idx) => (p.mark ? [`#${idx} "${p.title}" (owner ${p.owner_role}): ${p.mark}`] : []));
    return [
      `<case minutes="${c.minutes_id}">`,
      `<question>${c.minutes.question}</question>`,
      `<answer>${JSON.stringify({ answer: a.answer, priorities: a.priorities, concerns: a.concerns })}</answer>`,
      `<check_failures>\n${list(c.log.synthesis.first_check.failures)}\n</check_failures>`,
      `<unresolved_sources>${srcs(c.log.agents[o.agent]!.unresolved_sources)}</unresolved_sources>`,
      `<invented_sources>${srcs(c.log.synthesis.invented_sources)}</invented_sources>`,
      `<marks>\n${list(marks)}\n</marks>`,
      "</case>",
    ].join("\n");
  });
  return [
    ...(o.briefXml ? [o.briefXml, ""] : []),
    `<agent id="${o.agent}" role="${o.target.role}" target="${o.target.path}"${o.target.shared ? ` shared_lesson="${o.target.shared.text}"` : ""}/>`,
    `<trigger>${trigger}</trigger>`,
    `<effective_prompt>\n${o.effective}\n</effective_prompt>`,
    `<editable>\n${o.target.editable || "(empty: a new overlay)"}\n</editable>`,
    "<traces>",
    ...cases,
    "</traces>",
    "<earlier_proposals>",
    list(o.history.map((h) => `${h.id} ${h.status}: ${h.rationale}`)),
    "</earlier_proposals>",
  ].join("\n");
}

const pct = (x: number) => `${Math.round(x * 100)}%`;
const usd = (x: number) => `$${x.toFixed(3)}`;
const caseLine = (m: CaseMetrics) =>
  `check ${m.first_pass ? "ok" : "falla"}, sin resolver ${m.unresolved}, inventadas ${m.invented}, objetivos faltantes ${m.missing_objectives}, sin datos ${m.sin_datos}, costo ${usd(m.cost_usd)}`;
const totalPair = (b: ReplayTotals, a: ReplayTotals) =>
  [
    `check al primer intento ${pct(b.first_pass_rate)} → ${pct(a.first_pass_rate)}`,
    `sin resolver por caso ${b.unresolved_per_case.toFixed(2)} → ${a.unresolved_per_case.toFixed(2)}`,
    `inventadas ${b.invented} → ${a.invented}`,
    `objetivos faltantes ${b.missing_objectives} → ${a.missing_objectives}`,
    `sin datos ${b.sin_datos} → ${a.sin_datos}`,
    `costo ${usd(b.cost_usd)} → ${usd(a.cost_usd)}`,
  ].join(", ");

/** The replay table as text: one line per held-out case (before → after), then the totals. */
export function replayTable(r: ReplayReport): string[] {
  if (!r.totals) return [`Replay sin comparación: ${r.aborted ?? "sin casos"}.`];
  return [
    `Replay sobre ${r.totals.after.cases} casos held-out (antes → después):`,
    ...r.rows.map((row) => `- ${row.minutes_id}${row.baseline === "rerun" ? " (baseline re-corrido)" : ""}: ${caseLine(row.before)} → ${caseLine(row.after)}`),
    `Totales: ${totalPair(r.totals.before, r.totals.after)}.`,
  ];
}

export function decisionText(p: Proposal, reflectionId: string): { text: string; why: string } {
  const where = p.target.startsWith("plugin/") ? `${p.target}, el rol global de todos los productos` : `${p.target}${p.target_hash === null ? ", overlay nuevo" : ""}`;
  return {
    text: [`Cambiar el prompt de ${p.agent} (${where}) según la reflexión ${reflectionId}. Efecto esperado: ${p.expected_effect}`, ...replayTable(p.replay)].join("\n"),
    why: p.rationale,
  };
}

export interface ProposeInput {
  product: Product;
  root: string;
  reflectionId: string;
  log: MeetingLogEntry[];
  minutes: MarkedMinutes[];
  reflections: ReflectionEntry[];
  /** The agent's memory after this reflection's curation. */
  items: (agent: string) => Promise<MemoryItem[]>;
  warn: (msg: string) => void;
}

export interface ProposeDeps {
  call: ClaudeCaller;
  check?: (m: Parameters<typeof check>[0], opts: CheckOptions) => Promise<CheckResult>;
  resolve?: (s: Source) => Promise<boolean>;
  budgetUsd?: number;
}

export interface ProposeResult {
  candidates: ProposalCandidate[];
  proposal: Proposal | null;
  cost_usd: number;
}

async function readBriefSafe(product: string): Promise<Brief | null> {
  try {
    return await readBrief(product);
  } catch {
    return null;
  }
}

/**
 * For the agent with the strongest evidence past the threshold and enough replay cases: one proposal call reads its
 * tuning traces, code renders the change, replay judges it on the held-out half. A passing change becomes a pending
 * proposal, still without its decision; anything else is `discarded` with its numbers. Writes nothing.
 */
export async function proposeForReady(i: ProposeInput, d: ProposeDeps): Promise<ProposeResult> {
  const p = i.product.id;
  const candidates: ProposalCandidate[] = [];
  let chosen: { ready: ReadyAgent; tuning: ReplayCase[]; heldOut: ReplayCase[]; candidate: ProposalCandidate } | null = null;
  for (const ready of readyForProposal(i.log, i.minutes, i.reflections)) {
    const split = splitCases(await replayCases(i.root, p, ready.agent, i.log));
    const note = insufficientCases(split);
    const candidate: ProposalCandidate = {
      agent: ready.agent,
      reason: ready.reason,
      failure_class: ready.failure_class,
      count: ready.count,
      cases: split.tuning.length + split.heldOut.length,
      held_out: split.heldOut.length,
      outcome: note ? "insufficient_evidence" : chosen ? "deferred" : "proposed",
      note,
    };
    candidates.push(candidate);
    if (candidate.outcome === "proposed") chosen = { ready, ...split, candidate };
  }
  if (!chosen) return { candidates, proposal: null, cost_usd: 0 };

  const { agent } = chosen.ready;
  const budget = new Budget(d.budgetUsd ?? REPLAY_BUDGET_USD);
  const report = (run: Partial<ReplayRun>): ReplayReport => ({
    tuning: chosen.tuning.map((c) => c.minutes_id),
    held_out: chosen.heldOut.map((c) => c.minutes_id),
    rows: [],
    totals: null,
    verdict: { passed: false, worse: [], better: [] },
    aborted: null,
    ...run,
    cost_rule: COST_RULE,
    cost_usd: budget.spent,
    budget_usd: budget.limit,
  });

  let proposal: Proposal;
  try {
    const dirs = roleDirsAt(i.root);
    const items = await i.items(agent);
    const current = await loadRole(agent, p, dirs);
    const target = await chooseTarget(i.root, p, current, items);
    const brief = await readBriefSafe(p);
    const owner = loadOwner(i.root);
    const history = i.reflections.flatMap((e) => (e.proposal?.agent === agent ? [{ id: e.id, status: e.proposal.status, rationale: e.proposal.rationale ?? "" }] : []));
    const res = await budget.run(d.call, {
      prompt: proposalPrompt({ briefXml: brief ? renderBriefXml(brief) : null, agent, target, trigger: chosen.ready, effective: current.body, tuning: chosen.tuning, history }),
      system: proposalSystem(target.global, owner),
      model: PROPOSAL_MODEL,
      effort: "high",
      tools: [],
      schema: proposalSchema(target.global),
      cwd: i.root,
    });
    const out = (typeof res.output === "object" && res.output !== null ? res.output : {}) as Record<string, unknown>;
    const text = (k: string) => (typeof out[k] === "string" ? (out[k] as string).trim() : "");
    const rendered = renderProposal(target, text(bodyKey(target.global)));
    const content = "content" in rendered ? rendered.content : "";
    const bodyHash = content ? sha256(normalizeLesson(content)) : "";
    const diff = content ? await unifiedDiff((target.current ?? "").trimEnd(), content.trimEnd(), target.path) : "";
    const repeated = bodyHash
      ? i.reflections.find((e) => e.proposal?.target === target.path && e.proposal.body_hash === bodyHash && ["rejected", "discarded"].includes(e.proposal.status))
      : undefined;
    const aborted =
      "error" in rendered
        ? `propuesta inválida: ${rendered.error}`
        : !diff
          ? "la propuesta no cambia el prompt"
          : repeated
            ? `repite la propuesta de ${repeated.id} (${repeated.proposal!.status})`
            : null;

    const base = {
      agent,
      target: target.path,
      trigger: { reason: chosen.ready.reason, failure_class: chosen.ready.failure_class, count: chosen.ready.count },
      shared_lesson: target.shared,
      rationale: text("rationale"),
      expected_effect: text("expected_effect"),
      diff,
      content,
      body_hash: bodyHash,
      target_hash: await targetHash(target.path, i.root),
      decision_id: null,
    };
    if (aborted) {
      proposal = { ...base, replay: report({ aborted }), status: "discarded" };
    } else {
      const ctx = await sourceContext(i.product);
      const run = await replay(
        {
          product: i.product,
          agent,
          candidate: await effectiveWith(i.root, p, agent, target, content),
          current,
          pm: agent === SYNTHESIS_AGENT ? current : await loadRole(SYNTHESIS_AGENT, p, dirs),
          memory: renderForPrompt(items, MEMORY_MAX_CHARS),
          brief,
          owner,
          heldOut: chosen.heldOut,
          budget,
        },
        { call: d.call, check: d.check ?? check, resolve: d.resolve ?? ((s) => resolveSource(s, i.product, ctx)) },
      );
      proposal = { ...base, replay: report(run), status: run.verdict.passed ? "pending" : "discarded" };
    }
  } catch (err) {
    i.warn(`reflect ${agent}: proposal failed: ${(err as Error).message}`);
    chosen.candidate.outcome = "failed";
    chosen.candidate.note = (err as Error).message;
    return { candidates, proposal: null, cost_usd: budget.spent };
  }
  return { candidates, proposal, cost_usd: budget.spent };
}
