import { existsSync } from "node:fs";
import { resolveSource, sourceContext } from "./check.ts";
import { readBrief, renderBriefXml, type Brief } from "./brief.ts";
import { claude, type ClaudeCaller } from "./claude.ts";
import { compactSnapshot } from "./meeting.ts";
import { memoryForPrompt } from "./memory.ts";
import { getPool } from "./db.ts";
import { loadOwner, type Owner, type Product, type StoryConfig } from "./registry.ts";
import { loadRole, roleDirsAt } from "./roles.ts";
import { snapshot } from "./snapshot.ts";
import { listStories, loadStory, nextStoryId, requestChanges, saveStory, storySlug, validateStory, type Criterion, type Story, type StoryType } from "./story.ts";
import { storyTransaction } from "./story-write.ts";
import type { Source, UsageRow } from "./types.ts";

const PM_TOOLS = ["Read", "Grep", "Glob"];
const PM_TIMEOUT_MS = 8 * 60_000;
const MAX_SUGGESTIONS = 3;

/** What the model writes. Code sets id, slug, status, approval, proposed_by and the timestamps. */
export interface StoryContent {
  type: StoryType;
  title: string;
  as_a: string | null;
  i_want: string;
  so_that: string;
  criteria: Criterion[];
  owner_role: string;
  objective: string | null;
  sources: Source[];
}

export interface StoryProposeContext {
  product: Product;
  config: StoryConfig;
  /** AgentSI checkout that holds the workspace; tests pass a temp dir. */
  root: string;
  /** The model call. Tests pass a fake. */
  caller?: ClaudeCaller;
  /** The snapshot text. Tests pass a fixed string. */
  snapshot?: () => Promise<string>;
}

export interface StoryProposeResult {
  stories: Story[];
  usage: UsageRow[];
  totalCostUsd: number;
}

const SOURCE_SCHEMA = {
  type: "object",
  properties: {
    type: { type: "string", enum: ["commit", "issue", "path", "url", "story"] },
    ref: { type: "string" },
  },
  required: ["type", "ref"],
};

const CRITERION_SCHEMA = {
  type: "object",
  properties: {
    given: { type: "string" },
    when: { type: "string" },
    then: { type: "string" },
    test: { type: ["string", "null"] },
    checks: { type: "string" },
  },
  required: ["given", "when", "then", "test", "checks"],
};

export function storyContentSchema(roster: string[], brief: Brief | null): object {
  return {
    type: "object",
    properties: {
      stories: {
        type: "array",
        items: {
          type: "object",
          properties: {
            type: { type: "string", enum: ["story", "task"] },
            title: { type: "string" },
            as_a: { type: ["string", "null"] },
            i_want: { type: "string" },
            so_that: { type: "string" },
            criteria: { type: "array", items: CRITERION_SCHEMA },
            owner_role: { type: "string", enum: roster },
            objective: brief ? { enum: [...brief.objectives.map((o) => o.id), null] } : { type: "null" },
            sources: { type: "array", items: SOURCE_SCHEMA },
          },
          required: ["type", "title", "as_a", "i_want", "so_that", "criteria", "owner_role", "objective", "sources"],
        },
      },
    },
    required: ["stories"],
  };
}

const openStories = (stories: Story[]): string =>
  stories
    .filter((s) => s.status === "not_implemented")
    .map((s) => `- ${s.id} "${s.title}" (${s.type}, owner ${s.owner_role}, approval ${s.approval}, session ${s.session?.state ?? "none"})`)
    .join("\n") || "_None._";

export interface StoryProposeRequest {
  mode: "new" | "suggest" | "changes";
  /** `new`: the owner's text. `suggest`: empty. `changes`: the owner's note. */
  text: string;
  type?: StoryType;
  /** `changes` only: the story to rewrite. */
  current?: Story;
}

export function proposePrompt(o: {
  request: StoryProposeRequest;
  ctx: Pick<StoryProposeContext, "product" | "config">;
  snapshot: string;
  memory: string;
  brief: Brief | null;
  stories: Story[];
  owner: Owner;
  repair?: { previous: unknown; failures: string[] };
}): string {
  const { request: r, ctx, owner } = o;
  const task =
    r.mode === "new"
      ? `A story request from ${owner.name}. Shape the text into exactly one story:\n\n${r.text}`
      : r.mode === "suggest"
        ? `Propose up to ${MAX_SUGGESTIONS} stories from the snapshot, for any role of the roster. Fewer is better than weak ones. Skip anything an open story already covers.`
        : `Changes requested by ${owner.name} for the story below. Rewrite that one story with the note applied. Keep what the note does not touch.\n\nNote: ${r.text}\n\nCurrent story:\n${JSON.stringify(storyContent(r.current!))}`;
  const rules = [
    `- Write the story content only. Return it in "stories": ${r.mode === "suggest" ? `0 to ${MAX_SUGGESTIONS} items` : "exactly 1 item"}.`,
    r.type ? `- The type is "${r.type}".` : `- type is "story" when a user sees the change, "task" for debt, infra or a refactor. Prefer "task" for those.`,
    `- A story has as_a (the user), i_want and so_that. A task has as_a null.`,
    `- Everything the user reads is in ${owner.language}: title, as_a, i_want, so_that, given, when, then and checks. Identifiers, paths and code stay in English.`,
    `- title is short and says what the user gets. It never contains an id.`,
    `- Write 2 to 5 criteria. Each has one given, one when and one then.`,
    `- Each criterion has a test: the repo-relative path of a test file QA will write. It exercises behaviour the user sees (CLI output, an HTTP response, a rendered page), not an internal function. Use a path that the test command runs: \`${ctx.config.test}\`. Read the repo (Glob, Read) to follow its test layout. A task may use null.`,
    `- checks is one plain sentence in ${owner.language} of what the test proves.`,
    `- owner_role is one of the roster: ${ctx.product.roles.join(", ")}.`,
    o.brief ? `- objective is the id of the brief objective the story serves (${o.brief.objectives.map((x) => x.id).join(", ")}), or null.` : `- objective is null.`,
    `- sources: cite only commits, issues, files, URLs or open stories (type "story", ref its id) that exist. Check a path with Glob before you cite it. Use an empty list when unsure.`,
    `- Do not propose what ${owner.name} already rejected or what an open story covers.`,
    `- Working directory: ${ctx.config.repo} (read-only: Read, Grep, Glob). Cite paths relative to it.`,
  ];
  const out = [
    ...(o.brief ? [`# Brief of the product\n${renderBriefXml(o.brief)}`] : []),
    `# Snapshot of the product (compact)\n${o.snapshot}`,
    `# Open stories\n${openStories(o.stories)}`,
    `# Your memory about this product\n${o.memory || "_Empty._"}`,
    `# Task\n${task}`,
    `# Instructions\n${rules.join("\n")}`,
  ];
  if (o.repair) {
    out.push(
      `# Repair\nYour previous output failed these checks. Fix them and return the full corrected output.\n\nFailures:\n${o.repair.failures.map((f) => `- ${f}`).join("\n")}\n\nPrevious output:\n${JSON.stringify(o.repair.previous)}`,
    );
  }
  return out.join("\n\n");
}

const storyContent = (s: Story): StoryContent => ({
  type: s.type,
  title: s.title,
  as_a: s.as_a,
  i_want: s.i_want,
  so_that: s.so_that,
  criteria: s.criteria,
  owner_role: s.owner_role,
  objective: s.objective,
  sources: s.sources,
});

interface Fixed {
  proposed_by: Story["proposed_by"];
  minutes: string | null;
  now: Date;
}

/** A story from model content: code sets the fields the model must not decide. */
function toStory(content: StoryContent, id: string, fixed: Fixed): Story {
  const at = fixed.now.toISOString();
  return {
    id,
    type: content.type,
    slug: storySlug(typeof content.title === "string" ? content.title : ""),
    title: content.title,
    as_a: content.as_a,
    i_want: content.i_want,
    so_that: content.so_that,
    criteria: content.criteria,
    owner_role: content.owner_role,
    objective: content.objective,
    sources: content.sources,
    status: "not_implemented",
    approval: "pending",
    changes_note: null,
    proposed_by: fixed.proposed_by,
    minutes: fixed.minutes,
    session: null,
    results: [],
    created_at: at,
    updated_at: at,
  };
}

function parseOutput(output: unknown, request: StoryProposeRequest): { items: StoryContent[]; failures: string[] } {
  const items = (output as { stories?: unknown } | null)?.stories;
  if (!Array.isArray(items)) return { items: [], failures: [`The output has no "stories" list.`] };
  const failures: string[] = [];
  const max = request.mode === "suggest" ? MAX_SUGGESTIONS : 1;
  if (items.length > max) failures.push(`"stories" has ${items.length} items; at most ${max}.`);
  if (request.mode !== "suggest" && items.length === 0) failures.push(`"stories" is empty; return exactly 1 story.`);
  return { items: items as StoryContent[], failures };
}

async function validateAll(items: StoryContent[], build: (c: StoryContent) => Story, request: StoryProposeRequest, roster: string[], known: string[], product: Product, root: string, repo: string): Promise<string[]> {
  const ctx = await sourceContext(product, root);
  // The story repo is searched too, even when it is not one of the product paths.
  if (existsSync(repo)) {
    if (!ctx.roots.includes(repo)) ctx.roots.push(repo);
    if (!ctx.repos.includes(repo)) ctx.repos.push(repo);
  }
  const out: string[] = [];
  for (const [i, c] of items.entries()) {
    const s = build(c);
    const prefix = items.length > 1 ? `stories[${i + 1}]: ` : "";
    const failures = validateStory(s, { roster });
    for (const src of Array.isArray(c.sources) ? c.sources : []) {
      if (src?.type === "story" && typeof src.ref === "string" && !known.includes(src.ref)) failures.push(`The story source "${src.ref}" is not a story of the product.`);
      if ((src?.type === "path" || src?.type === "commit") && typeof src.ref === "string" && !(await resolveSource(src, product, ctx))) {
        failures.push(`The ${src.type} source "${src.ref}" does not exist in the repo.`);
      }
    }
    const forced = request.mode === "changes" ? request.current!.type : request.type;
    if (forced && c.type !== forced) failures.push(`type must be "${forced}", found "${c.type}".`);
    out.push(...failures.map((f) => prefix + f));
  }
  return out;
}

/** One PM call, repaired once with the failure list; a second failure throws and writes nothing. */
async function propose(ctx: StoryProposeContext, request: StoryProposeRequest, fixed: Fixed, build: (c: StoryContent) => Story): Promise<{ items: StoryContent[]; usage: UsageRow[] }> {
  const { product, config, root } = ctx;
  const call = ctx.caller ?? claude;
  const pm = await loadRole("pm", product.id, roleDirsAt(root));
  const brief = await readBrief(product.id);
  const snap = compactSnapshot(await (ctx.snapshot ?? (() => snapshot(product.id)))());
  const stories = await listStories(product.id, root);
  const memory = await memoryForPrompt(product.id, "pm");
  const owner = loadOwner(root);
  const schema = storyContentSchema(product.roles, brief);
  const usage: UsageRow[] = [];

  const ask = async (repair?: { previous: unknown; failures: string[] }) => {
    const r = await call({
      prompt: proposePrompt({ request, ctx, snapshot: snap, memory, brief, stories, owner, repair }),
      model: pm.model,
      effort: pm.effort,
      system: `${pm.body}\n\nNow you shape stories for Stories mode. You write story content only.`,
      cwd: config.repo,
      tools: PM_TOOLS,
      schema,
      timeoutMs: PM_TIMEOUT_MS,
    });
    usage.push({ role: repair ? "pm-repair" : "pm", ...r.usage });
    return r.output;
  };
  const check = async (output: unknown) => {
    const parsed = parseOutput(output, request);
    const failures = [...parsed.failures, ...(await validateAll(parsed.items, build, request, product.roles, stories.map((s) => s.id), product, root, config.repo))];
    return { items: parsed.items, failures };
  };

  const first = await ask();
  let result = await check(first);
  if (result.failures.length) {
    const output = await ask({ previous: first, failures: result.failures });
    result = await check(output);
    if (result.failures.length) throw new Error(`The PM output is invalid after one repair, so nothing was written:\n${result.failures.map((f) => `- ${f}`).join("\n")}`);
  }
  return { items: result.items, usage };
}

const total = (usage: UsageRow[]) => usage.reduce((n, u) => n + u.cost_usd, 0);

/** Saves the stories one by one so each gets the next id, in one story transaction. */
function saveAll(ctx: StoryProposeContext, items: StoryContent[], fixed: Fixed): Promise<Story[]> {
  const save = async () => {
    const saved: Story[] = [];
    for (const c of items) saved.push(await saveStory(ctx.product, toStory(c, await nextStoryId(ctx.product.id, c.type, ctx.root), fixed), ctx.root));
    return saved;
  };
  return storyTransaction(ctx, save);
}

/** The id only feeds the validator; `saveAll` assigns the real one. */
const provisional = async (ctx: StoryProposeContext, fixed: Fixed): Promise<(c: StoryContent) => Story> => {
  const ids = { story: await nextStoryId(ctx.product.id, "story", ctx.root), task: await nextStoryId(ctx.product.id, "task", ctx.root) };
  return (c) => toStory(c, c.type === "task" ? ids.task : ids.story, fixed);
};

async function assertMinutes(ctx: StoryProposeContext, minutes: string | null): Promise<void> {
  if (minutes === null) return;
  const found = /^[\w.-]+$/.test(minutes) && (await getPool().query("select 1 from minutes where id = $1 and product = $2 and data is not null", [minutes, ctx.product.id])).rows.length > 0;
  if (!found) throw new Error(`Unknown minutes "${minutes}" in ${ctx.product.id}`);
}

/** `story new`: the PM shapes the owner's text into one story. */
export async function proposeStory(ctx: StoryProposeContext, o: { text: string; type?: StoryType; minutes?: string }): Promise<StoryProposeResult> {
  if (!o.text.trim()) throw new Error("A story needs a text.");
  const minutes = o.minutes ?? null;
  await assertMinutes(ctx, minutes);
  const fixed: Fixed = { proposed_by: "owner", minutes, now: new Date() };
  const { items, usage } = await propose(ctx, { mode: "new", text: o.text.trim(), type: o.type }, fixed, await provisional(ctx, fixed));
  return { stories: await saveAll(ctx, items, fixed), usage, totalCostUsd: total(usage) };
}

/** `story suggest`: up to 3 stories from the snapshot, proposed by the PM. */
export async function suggestStories(ctx: StoryProposeContext): Promise<StoryProposeResult> {
  const fixed: Fixed = { proposed_by: "pm", minutes: null, now: new Date() };
  const { items, usage } = await propose(ctx, { mode: "suggest", text: "" }, fixed, await provisional(ctx, fixed));
  return { stories: items.length ? await saveAll(ctx, items, fixed) : [], usage, totalCostUsd: total(usage) };
}

/** `story changes`: the PM rewrites the story with the note; id, slug and history stay. */
export async function reviseStory(ctx: StoryProposeContext, id: string, note: string): Promise<StoryProposeResult> {
  const current = await loadStory(ctx.product.id, id, ctx.root);
  const asked = requestChanges(current, note);
  const fixed: Fixed = { proposed_by: current.proposed_by, minutes: current.minutes, now: new Date() };
  const build = (c: StoryContent) => ({ ...toStory(c, current.id, fixed), slug: current.slug });
  const { items, usage } = await propose(ctx, { mode: "changes", text: asked.changes_note!, current }, fixed, build);
  const save = async () => {
    // The model call ran outside the lock, so the story must still accept the change.
    const fresh = requestChanges(await loadStory(ctx.product.id, id, ctx.root), note);
    return saveStory(ctx.product, { ...fresh, ...storyContent({ ...fresh, ...items[0]! }), approval: "pending" }, ctx.root);
  };
  const saved = await storyTransaction(ctx, save);
  return { stories: [saved], usage, totalCostUsd: total(usage) };
}
