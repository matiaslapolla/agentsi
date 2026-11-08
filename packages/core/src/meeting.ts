import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { briefHash, readBrief, renderBriefXml, type Brief } from "./brief.ts";
import { check, resolveSource, sourceContext } from "./check.ts";
import { claude, totalInputTokens, type ClaudeCall } from "./claude.ts";
import { codeGraphMcp, reindexCodeGraph } from "./code-graph.ts";
import { indexProduct } from "./indexer.ts";
import { NO_DATA, buildLogEntry, type AgentAnswerLog } from "./meeting-log.ts";
import { memoryForPrompt } from "./memory.ts";
import { findMinutes, newMinutesId, reserveMinutesId, writeMinutes } from "./minutes.ts";
import { audioDir, codeRoot, dataRoot, workspaceRoot } from "./paths.ts";
import { getProduct, loadRegistry, ownerOf, type Owner, type Product } from "./registry.ts";
import { loadRole, roleDirsAt, type RoleDef } from "./roles.ts";
import { repoTargets } from "./repos.ts";
import { snapshot } from "./snapshot.ts";
import type { Minutes, RoleAnswer, RolePriority, UsageRow } from "./types.ts";

export interface MeetingOptions {
  product: string;
  scope?: string | null;
  question: string;
  roles?: string[];
  onEvent?: (e: MeetingEvent) => void;
}

export interface MeetingEvent {
  stage: "index" | "role" | "synthesis" | "check";
  phase: "start" | "end";
  role?: string;
  seconds?: number;
  cost_usd?: number;
  input_tokens?: number;
  error?: string;
  note?: string;
}

export interface MeetingResult {
  minutes: Minutes;
  usage: UsageRow[];
  totalCostUsd: number;
}

export interface AskResult {
  answer: string;
  usage: UsageRow;
}

const ROLE_TIMEOUT_MS = 6 * 60_000;
const BASE_TOOLS = ["Read", "Grep", "Glob"];

const SOURCE_SCHEMA = {
  type: "object",
  properties: {
    type: { type: "string", enum: ["commit", "issue", "path", "url", "story"] },
    ref: { type: "string" },
  },
  required: ["type", "ref"],
};

const strings = { type: "array", items: { type: "string" } };

export const roleAnswerSchema = {
  type: "object",
  properties: {
    answer: { type: "string" },
    priorities: {
      type: "array",
      items: {
        type: "object",
        properties: {
          title: { type: "string" },
          why: { type: "string" },
          sources: { type: "array", items: SOURCE_SCHEMA },
        },
        required: ["title", "why", "sources"],
      },
    },
    concerns: strings,
    memory_notes: strings,
  },
  required: ["answer", "priorities", "concerns", "memory_notes"],
};

/** With a brief, every priority must name one of its objectives. */
export const synthesisSchemaFor = (brief: Brief | null) => ({
  type: "object",
  properties: {
    priorities: {
      type: "array",
      items: {
        type: "object",
        properties: {
          title: { type: "string" },
          why: { type: "string" },
          sources: { type: "array", items: SOURCE_SCHEMA },
          owner_role: { type: "string" },
          ...(brief ? { objective: { type: "string", enum: brief.objectives.map((o) => o.id) } } : {}),
        },
        required: ["title", "why", "sources", "owner_role", ...(brief ? ["objective"] : [])],
      },
    },
    disagreements: {
      type: "array",
      items: {
        type: "object",
        properties: {
          topic: { type: "string" },
          positions: {
            type: "array",
            items: {
              type: "object",
              properties: { role: { type: "string" }, view: { type: "string" } },
              required: ["role", "view"],
            },
          },
        },
        required: ["topic", "positions"],
      },
    },
    decisions: {
      type: "array",
      items: {
        type: "object",
        properties: { text: { type: "string" }, why: { type: "string" }, action: { type: "string" }, priorities: { type: "array", items: { type: "integer" } } },
        required: ["text", "why"],
      },
    },
    script: {
      type: "array",
      items: {
        type: "object",
        properties: { role: { type: "string" }, text: { type: "string" } },
        required: ["role", "text"],
      },
    },
  },
  required: ["priorities", "disagreements", "decisions", "script"],
});

export interface Synthesis {
  priorities: { title: string; why: string; sources: RolePriority["sources"]; owner_role: string; objective?: string }[];
  disagreements: Minutes["disagreements"];
  decisions: { text: string; why: string; action?: string; priorities?: number[] }[];
  script: Minutes["script"];
}

export function productCwd(product: Product, scope: string | null): string {
  const target = repoTargets(product, scope)[0];
  const dir = target?.path ?? product.paths.find((p) => existsSync(p));
  if (!dir) throw new Error(`No existing path for product "${product.id}"`);
  return dir;
}

const pathRule = (scope: string | null) =>
  scope
    ? `Cite file paths relative to the product root, so prefix them with "${scope}/" (for example "${scope}/docs/x.md").`
    : "Cite file paths relative to the working directory.";

export function rolePrompt(o: {
  question: string;
  snapshot: string;
  memory: string;
  cwd: string;
  scope: string | null;
  role: string;
  web: boolean;
  meeting: boolean;
  brief: Brief | null;
  owner: Owner;
  /** Code graph project names; empty when the role has no code graph tools. */
  codeGraph?: string[];
}): string {
  const rules = [
    `- You are the ${o.role} of the agency. Working directory: ${o.cwd} (read-only: Read, Grep, Glob${o.web ? ", WebSearch" : ""}).`,
    `- Answer in ${o.owner.language}, short and direct. Keep identifiers, paths and code in English.`,
    `- Base every claim on the snapshot, your memory or files you read. Write "${NO_DATA}" and name what is missing; never invent numbers, users, metrics or dates.`,
    `- You only give opinions. Each change you propose is a decision ${o.owner.name} must approve. One priority you can defend beats five vague ones.`,
  ];
  if (o.codeGraph?.length) {
    rules.push(
      `- The code graph tools (mcp__codegraph__*, projects: ${o.codeGraph.join(", ")}) find symbols, callers and call paths fast. Read the file to confirm before you cite it.`,
    );
  }
  if (o.meeting) {
    rules.push(
      `- Give at most 3 priorities. Each cites at least one source of type commit (hash from the snapshot), issue (number), path (a file that exists), url (full http(s) URL${o.web ? ", for web pages" : ""}) or story (an id from the Stories section of the snapshot). ${pathRule(o.scope)}`,
      o.brief
        ? `- Cite the north stars declared in the <brief> (<north_stars>) where they support a priority. Keep everything under <out_of_scope> out of your priorities.`
        : `- Cite the product's north-star or guideline docs (CLAUDE.md, CONTEXT.md, PRODUCT.md, DESIGN.md, README.md, docs/...) where they support a priority.`,
      `- "answer" is your view in at most 150 words. "concerns": at most 3 risks you see. "memory_notes": only facts about this product still true next week (a goal, a constraint, a decision, a recurring risk), not opinions about today; usually empty.`,
    );
  } else {
    rules.push(`- Cite sources inline: commit hashes, issue numbers or repo-relative paths. Plain text, no JSON.`);
  }
  // Stable context first, so repeated calls on the same product share a cacheable prefix.
  return [
    ...(o.brief ? [`# Brief of the product\n${renderBriefXml(o.brief)}`] : []),
    `# Snapshot of the product\n${o.snapshot}`,
    `# Your memory about this product\n${o.memory || "_Empty: you have not met on this product yet._"}`,
    `# Question\n${o.question}`,
    `# Instructions\n${rules.join("\n")}`,
  ].join("\n\n");
}

/** Keeps the sections of the snapshot that the synthesis needs, within a byte budget. */
export function compactSnapshot(full: string, maxChars = 14_000): string {
  // Only the snapshot's own headings split it: the north-star docs and the journal carry `## ` headings of their own.
  const sections = full.split(/\n(?=## (?:North star|Open issues|Last minutes|Pending decisions|Stories|Commits|Journal tail|Docs index|Database sections))/);
  const keep = ["# Snapshot", "## North star", "## Pending decisions", "## Last minutes", "## Stories", "## Commits", "## Docs index"];
  const limits: Record<string, number> = { "## North star": 5000, "## Stories": 1500, "## Commits": 2500, "## Docs index": 3500 };
  const out: string[] = [];
  for (const s of sections) {
    const k = keep.find((p) => s.startsWith(p));
    if (!k) continue;
    const limit = limits[k];
    out.push(limit && s.length > limit ? `${s.slice(0, limit)}\n[cut]` : s);
  }
  return out.join("\n\n").slice(0, maxChars);
}

export interface RoleContext {
  question: string;
  snap: string;
  memory: string;
  scope: string | null;
  cwd: string;
  brief: Brief | null;
  owner: Owner;
  /** Code graph project names reindexed for this meeting. */
  codeGraph?: string[];
}

/** The call that asks one agent; `meeting` asks for the structured answer of a meeting. */
export function roleCall(def: RoleDef, ctx: RoleContext, meeting: boolean): ClaudeCall {
  const tools = def.tools ?? BASE_TOOLS;
  const codeGraph = tools.length ? (ctx.codeGraph ?? []) : [];
  const prompt = rolePrompt({
    question: ctx.question,
    snapshot: ctx.snap,
    memory: ctx.memory,
    cwd: ctx.cwd,
    scope: ctx.scope,
    role: def.id,
    web: tools.includes("WebSearch"),
    meeting,
    brief: ctx.brief,
    owner: ctx.owner,
    codeGraph,
  });
  return {
    prompt,
    model: def.model,
    effort: def.effort,
    system: def.body,
    cwd: ctx.cwd,
    tools,
    ...(codeGraph.length ? { mcp: codeGraphMcp() } : {}),
    schema: meeting ? roleAnswerSchema : undefined,
    timeoutMs: ROLE_TIMEOUT_MS,
  };
}

/** Normalizes a structured role output into an answer. */
export function toRoleAnswer(output: unknown, model: string): RoleAnswer {
  const a = (output ?? {}) as Partial<RoleAnswer>;
  return { model, answer: a.answer ?? "", priorities: a.priorities ?? [], concerns: a.concerns ?? [], memory_notes: a.memory_notes ?? [] };
}

export function emptyAnswer(model: string, error: string): RoleAnswer {
  return { model, answer: "", priorities: [], concerns: [`Role call failed: ${error}`], memory_notes: [] };
}

export function synthesisPrompt(
  question: string,
  answers: Record<string, RoleAnswer>,
  compact: string,
  brief: Brief | null,
  owner: Owner,
  repair?: { previous: Synthesis; failures: string[] },
): string {
  const roleText = Object.entries(answers)
    .map(([r, a]) => `## ${r}\n\n${JSON.stringify({ answer: a.answer, priorities: a.priorities, concerns: a.concerns })}`)
    .join("\n\n");
  const out = [
    `# Question\n${question}`,
    `# Agent answers\n${roleText || "_No agent answered._"}`,
    ...(brief ? [`# Brief of the product\n${renderBriefXml(brief)}`] : []),
    `# Snapshot (compact)\n${compact}`,
    [
      `# Instructions`,
      `- Rank at most 5 priorities. Each cites at least one source, copied verbatim (type and ref) from the agent answers or the snapshot. Never invent a source.`,
      ...(brief
        ? [
            `- "objective" is the id of the brief objective the priority serves: one of ${brief.objectives.map((o) => o.id).join(", ")}.`,
            ...(brief.north_stars.length
              ? [`- At least one priority cites a declared north star as a path source (${brief.north_stars.join(", ")}). Cite only those as north stars.`]
              : []),
            `- Keep everything under <out_of_scope> out of the priorities.`,
          ]
        : [
            `- At least one priority cites a north-star or guideline doc as a path source (CLAUDE.md, CONTEXT.md, PRODUCT.md, DESIGN.md, README.md or a file under docs/). Use the paths exactly as the agent answers or the snapshot give them.`,
          ]),
      `- "owner_role" is the id of the agent that proposed the priority most strongly, exactly as it heads its answer.`,
      `- "disagreements": only real conflicts between agents; an empty array when there are none. Each position's "role" is an agent id.`,
      `- "decisions": things that need the approval of ${owner.name} before anything changes. Each with its reason and an "action": what an agent would do after the approval, in one sentence. Each also lists in "priorities" the indexes (starting at 0, in the order of your "priorities") of the priorities it covers; an empty array when it covers none. Approving the decision marks those priorities accepted.`,
      `- "script": a team call in ${owner.language}, written to be spoken, in ASD-STE100 Simplified Technical English style applied to ${owner.language}: one topic in each sentence, at most 20 words in a sentence, the active voice with a named agent, simple verb forms, the condition first, one name for each thing. No slogans, metaphors, filler or fragments for effect. The pm opens by stating the question. Each agent that answered speaks its own view in 2 to 4 sentences, in first person. Refer to commits and files by what they are, never read hashes or paths aloud. Name a story by its title, never by its id. Add a short back-and-forth where agents disagree. The pm closes with the ranked priorities and the decisions ${owner.name} must take. Total about 2 to 4 minutes of speech. No markdown, no emojis. The "role" of each segment is the id of one of the agents that answered, or pm.`,
      `- Everything in ${owner.language} except identifiers, paths and code.`,
    ].join("\n"),
  ];
  if (repair) {
    out.push(
      `# Repair\nYour previous output failed these checks. Fix them and return the full corrected output.\n\nFailures:\n${repair.failures.map((f) => `- ${f}`).join("\n")}\n\nPrevious output:\n${JSON.stringify(repair.previous)}`,
    );
  }
  return out.join("\n\n");
}

/** The synthesis call, made with the product's pm agent as its system prompt. */
export function synthesisCall(
  pm: RoleDef,
  question: string,
  answers: Record<string, RoleAnswer>,
  compact: string,
  cwd: string,
  brief: Brief | null,
  owner: Owner,
  repair?: { previous: Synthesis; failures: string[] },
): ClaudeCall {
  return {
    prompt: synthesisPrompt(question, answers, compact, brief, owner, repair),
    model: pm.model,
    effort: pm.effort,
    system: `${pm.body}\n\nNow you write the minutes of the meeting from the agent answers.`,
    cwd,
    tools: [],
    schema: synthesisSchemaFor(brief),
    timeoutMs: ROLE_TIMEOUT_MS,
  };
}

type MinutesMeta = Pick<Minutes, "id" | "product" | "scope" | "question" | "created_at">;

/** Minutes from a synthesis (null when it failed), before `check()` fills the result. */
export function buildMinutes(meta: MinutesMeta, roles: Record<string, RoleAnswer>, s: Synthesis | null, repaired: boolean, firstFailures: string[] = []): Minutes {
  return {
    ...meta,
    roles,
    priorities: (s?.priorities ?? []).map((p) => ({ ...p, objective: p.objective ?? null, mark: null, is_new: false })),
    disagreements: s?.disagreements ?? [],
    decisions: s?.decisions ?? [],
    script: s?.script ?? [],
    check: { passed: false, failures: [], first_failures: firstFailures, repaired },
    status: "invalid",
  };
}

/** Every agent's definition and memory, and the pm that writes the synthesis; throws before any call is paid. */
export async function prepareAgents(product: string, roleNames: string[], root = dataRoot): Promise<{ agents: { def: RoleDef; memory: string }[]; pm: RoleDef }> {
  const dirs = roleDirsAt(root);
  const agents: { def: RoleDef; memory: string }[] = [];
  for (const id of roleNames) agents.push({ def: await loadRole(id, product, dirs), memory: await memoryForPrompt(product, id) });
  return { agents, pm: await loadRole("pm", product, dirs) };
}

export async function runMeeting(o: MeetingOptions): Promise<MeetingResult> {
  const emit = o.onEvent ?? (() => {});
  const registry = loadRegistry();
  const product = getProduct(o.product, registry);
  const owner = ownerOf(registry);
  const scope = o.scope ?? null;
  const roleNames = o.roles?.length ? o.roles : product.roles;
  const { agents, pm } = await prepareAgents(product.id, roleNames);
  const defs = agents.map((a) => a.def);
  const cwd = productCwd(product, scope);
  const brief = await readBrief(product.id);

  emit({ stage: "index", phase: "start" });
  await indexProduct(product.id);
  const codeGraph: string[] = [];
  for (const path of product.paths.filter((p) => existsSync(p))) {
    const project = await reindexCodeGraph(path);
    if (project) codeGraph.push(project);
  }
  const snap = await snapshot(product.id, scope);
  emit({ stage: "index", phase: "end" });

  const usage: UsageRow[] = [];
  const roles: Record<string, RoleAnswer> = {};
  const logs: Record<string, AgentAnswerLog> = {};
  await Promise.all(
    agents.map(async ({ def, memory }) => {
      const started = Date.now();
      logs[def.id] = { prompt: def.body, memory, model: def.model, error: null, duration_ms: 0, unresolved_sources: [] };
      emit({ stage: "role", phase: "start", role: def.id });
      try {
        const r = await claude(roleCall(def, { question: o.question, snap, memory, scope, cwd, brief, owner, codeGraph }, true));
        roles[def.id] = toRoleAnswer(r.output, r.usage.model);
        usage.push({ role: def.id, ...r.usage });
        logs[def.id]!.model = r.usage.model;
        emit({ stage: "role", phase: "end", role: def.id, seconds: (Date.now() - started) / 1000, cost_usd: r.usage.cost_usd, input_tokens: totalInputTokens(r.usage) });
      } catch (err) {
        roles[def.id] = emptyAnswer(def.model, (err as Error).message);
        logs[def.id]!.error = (err as Error).message;
        emit({ stage: "role", phase: "end", role: def.id, seconds: (Date.now() - started) / 1000, error: (err as Error).message });
      }
      logs[def.id]!.duration_ms = Date.now() - started;
    }),
  );
  const ordered: Record<string, RoleAnswer> = Object.fromEntries(defs.map((d) => [d.id, roles[d.id]!]));

  const sources = await sourceContext(product);
  for (const [name, a] of Object.entries(ordered)) {
    for (const s of a.priorities.flatMap((p) => p.sources)) {
      if (!(await resolveSource(s, product, sources))) logs[name]!.unresolved_sources.push(s);
    }
  }

  const compact = compactSnapshot(snap);
  const synth = async (role: string, repair?: { previous: Synthesis; failures: string[] }) => {
    const started = Date.now();
    emit({ stage: "synthesis", phase: "start", role, note: repair ? "repair" : undefined });
    try {
      const r = await claude(synthesisCall(pm, o.question, ordered, compact, cwd, brief, owner, repair));
      usage.push({ role, ...r.usage });
      emit({ stage: "synthesis", phase: "end", role, seconds: (Date.now() - started) / 1000, cost_usd: r.usage.cost_usd, input_tokens: totalInputTokens(r.usage) });
      return r.output as Synthesis;
    } catch (err) {
      emit({ stage: "synthesis", phase: "end", role, seconds: (Date.now() - started) / 1000, error: (err as Error).message });
      return null;
    }
  };

  const createdAt = new Date();
  const meta: MinutesMeta = { id: newMinutesId(o.question, createdAt), product: product.id, scope, question: o.question, created_at: createdAt.toISOString() };
  const build = (s: Synthesis | null, repaired: boolean, firstFailures: string[] = []): Minutes => buildMinutes(meta, ordered, s, repaired, firstFailures);
  const verify = async (m: Minutes) => {
    emit({ stage: "check", phase: "start" });
    const res = await check(m, { roles: roleNames, product, brief });
    m.check = { ...res, first_failures: m.check.first_failures, repaired: m.check.repaired };
    m.status = res.passed ? "valid" : "invalid";
    emit({ stage: "check", phase: "end", note: res.passed ? "passed" : `${res.failures.length} failures: ${res.failures.join(" | ")}` });
  };

  let synthesis = await synth("synthesis");
  let minutes = build(synthesis, false);
  await verify(minutes);
  minutes.check.first_failures = minutes.check.failures;
  if (!minutes.check.passed && synthesis) {
    const repaired = await synth("synthesis-repair", { previous: synthesis, failures: minutes.check.failures });
    if (repaired) {
      synthesis = repaired;
      minutes = build(repaired, true, minutes.check.first_failures);
      await verify(minutes);
    } else {
      minutes.check.repaired = true;
    }
  }

  // Two meetings that start in the same minute with the same question share an id until one claims it.
  minutes.id = await reserveMinutesId(minutes);
  const entry = buildLogEntry({ minutes, usage, agents: logs, snapshot: snap, briefHash: brief ? briefHash(brief) : null });
  await writeMinutes(minutes, usage, { entry, snapshot: snap });
  return {
    minutes,
    usage,
    totalCostUsd: usage.reduce((n, u) => n + u.cost_usd, 0),
  };
}

export async function askRole(o: { product: string; scope?: string | null; role: string; question: string }): Promise<AskResult> {
  const registry = loadRegistry();
  const product = getProduct(o.product, registry);
  const scope = o.scope ?? null;
  const def = await loadRole(o.role, product.id);
  const cwd = productCwd(product, scope);
  await indexProduct(product.id);
  const snap = await snapshot(product.id, scope);
  const brief = await readBrief(product.id);
  const r = await claude(roleCall(def, { question: o.question, snap, memory: await memoryForPrompt(product.id, def.id), scope, cwd, brief, owner: ownerOf(registry) }, false));
  return { answer: String(r.output).trim(), usage: { role: def.id, ...r.usage } };
}

/** The stored minutes of `id` in any product. Ids are global. */
export async function findStoredMinutes(id: string): Promise<Minutes> {
  if (!/^[\w.-]+$/.test(id)) throw new Error(`Invalid minutes id "${id}"`);
  const found = await findMinutes(id);
  if (!found) throw new Error(`Minutes "${id}" not found in any product`);
  return found;
}

/** Renders the call script of stored minutes into workspace/<product>/audio/<id>/. */
export function renderAudio(m: Pick<Minutes, "id" | "product" | "script">): Promise<string> {
  if (!m.script.length) return Promise.reject(new Error("Minutes have an empty script"));
  const out = audioDir(m.product, m.id);
  mkdirSync(out, { recursive: true });
  const tmp = mkdtempSync(join(tmpdir(), "agentsi-script-"));
  const scriptFile = join(tmp, "script.json");
  writeFileSync(scriptFile, JSON.stringify(m.script));
  return new Promise((resolve, reject) => {
    const child = spawn("uv", ["run", "--project", join(codeRoot, "packages/voice"), "agentsi-voice", "render", scriptFile, out], {
      stdio: ["ignore", "pipe", "pipe"],
    });
    let stderr = "";
    child.stderr.on("data", (d) => (stderr += d));
    child.on("error", (err) => {
      rmSync(tmp, { recursive: true, force: true });
      reject(err);
    });
    child.on("close", (code) => {
      rmSync(tmp, { recursive: true, force: true });
      if (code === 0) resolve(out);
      else reject(new Error(`agentsi-voice exited ${code}: ${stderr.trim().split("\n").slice(-4).join(" | ")}`));
    });
  });
}
