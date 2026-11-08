import { existsSync, mkdtempSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { isAbsolute, join } from "node:path";
import { isDeepStrictEqual } from "node:util";
import { deleteBrief, readBrief, writeBrief, type Brief } from "./brief.ts";
import { getPool, inTransaction } from "./db.ts";
import { commitFiles, git, type CommitResult } from "./exec.ts";
import { deleteOverlay, readOverlays, writeOverlay } from "./overlays.ts";
import { dataRoot } from "./paths.ts";
import { normalStories, storyConfig, type Product, type StoryConfigBlock } from "./registry.ts";
import { addProduct, editRegistryFile, updateProduct } from "./registry-edit.ts";
import { validateRoster } from "./roster.ts";

interface Stage {
  roster: string[];
  /** Stage-relative path to file text: `brief.json` and `agents/<id>.md`. */
  files: Record<string, string>;
  brief: Brief;
  /** Overlay text by agent id. */
  overlays: Record<string, string>;
  /** From `stories.json`: a block sets Stories mode, null turns it off, undefined leaves it. */
  stories: StoryConfigBlock | null | undefined;
}

export interface FileChange {
  path: string;
  status: "new" | "changed" | "removed";
  diff: string;
}

export interface ApplyPlan {
  product: Product;
  /** True when the product is not in the registry yet. */
  register: boolean;
  rosterBefore: string[];
  rosterAfter: string[];
  changes: FileChange[];
  storiesBefore?: StoryConfigBlock;
  storiesAfter?: StoryConfigBlock;
}

export interface ApplyInput {
  stageDir: string;
  /** The registered product, or the product to register (its `roles` are replaced by the stage roster). */
  product: Product;
  register: boolean;
  root?: string;
}

function readStage(dir: string): { stage: Stage | null; failures: string[] } {
  const failures: string[] = [];
  const text = (name: string) => (existsSync(join(dir, name)) ? readFileSync(join(dir, name), "utf8") : null);
  const brief = text("brief.json");
  const rosterText = text("roster.json");
  if (brief === null) failures.push("Stage has no brief.json.");
  if (rosterText === null) failures.push("Stage has no roster.json.");
  let roster: string[] = [];
  try {
    const parsed = rosterText === null ? [] : JSON.parse(rosterText);
    if (rosterText !== null && (!Array.isArray(parsed) || parsed.some((x) => typeof x !== "string"))) failures.push("roster.json must be an array of agent ids.");
    else roster = parsed;
  } catch (err) {
    failures.push(`roster.json is not valid JSON: ${(err as Error).message}`);
  }
  let parsedBrief: Brief | undefined;
  if (brief !== null) {
    try {
      parsedBrief = JSON.parse(brief) as Brief;
    } catch (err) {
      failures.push(`brief.json is not valid JSON: ${(err as Error).message}`);
    }
  }
  let stories: StoryConfigBlock | null | undefined;
  const storiesText = text("stories.json");
  if (storiesText !== null) {
    try {
      const parsed = JSON.parse(storiesText);
      if (parsed !== null && (typeof parsed !== "object" || Array.isArray(parsed))) failures.push("stories.json must be an object or null.");
      else stories = parsed;
    } catch (err) {
      failures.push(`stories.json is not valid JSON: ${(err as Error).message}`);
    }
  }
  const files: Record<string, string> = {};
  if (brief !== null) files["brief.json"] = brief;
  const overlays: Record<string, string> = {};
  const stageAgents = join(dir, "agents");
  if (existsSync(stageAgents)) {
    for (const f of readdirSync(stageAgents).filter((n) => n.endsWith(".md")).sort()) {
      files[`agents/${f}`] = readFileSync(join(stageAgents, f), "utf8");
      overlays[f.slice(0, -3)] = files[`agents/${f}`]!;
    }
  }
  return { stage: failures.length || !parsedBrief ? null : { roster, files, brief: parsedBrief, overlays, stories }, failures };
}

/** What the database holds for the product, as the stage files it would come from. A brief equal to the stage's keeps the stage's text. */
async function currentFiles(product: string, stage: Stage): Promise<Record<string, string>> {
  const files: Record<string, string> = {};
  const brief = await readBrief(product);
  if (brief) files["brief.json"] = isDeepStrictEqual(brief, stage.brief) ? stage.files["brief.json"]! : JSON.stringify(brief, null, 2) + "\n";
  for (const [id, text] of Object.entries(await readOverlays(product))) files[`agents/${id}.md`] = text;
  return files;
}

async function unifiedDiff(before: string, after: string): Promise<string> {
  const dir = mkdtempSync(join(tmpdir(), "agentsi-diff-"));
  try {
    writeFileSync(join(dir, "before"), before);
    writeFileSync(join(dir, "after"), after);
    const res = await git(dir, "diff", "--no-index", "--no-color", "--unified=2", "--", "before", "after");
    return res.stdout.split("\n").filter((l) => !/^(diff --git|index |--- |\+\+\+ )/.test(l)).join("\n").trim();
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

function storiesProblems(product: Product, block: StoryConfigBlock): string[] {
  const extra = Object.keys(block).filter((k) => !["repo", "test", "gate", "budget_usd"].includes(k));
  if (extra.length) return [`stories.json has unknown keys: ${extra.join(", ")}.`];
  if (block.repo !== undefined && (typeof block.repo !== "string" || !isAbsolute(block.repo) || !existsSync(block.repo) || !statSync(block.repo).isDirectory())) {
    return [`stories.json repo must be an existing absolute path to a directory, got "${block.repo}".`];
  }
  try {
    storyConfig({ ...product, stories: block });
  } catch (err) {
    return [(err as Error).message];
  }
  return [];
}

/** Validates the stage and compares it with the current workspace; nothing is written. */
export async function planApply(input: ApplyInput): Promise<{ plan: ApplyPlan | null; failures: string[] }> {
  const root = input.root ?? dataRoot;
  const { stage, failures } = readStage(input.stageDir);
  if (!stage) return { plan: null, failures };
  const storiesBefore = input.product.stories;
  const storiesAfter = stage.stories === undefined ? storiesBefore : stage.stories === null ? undefined : normalStories(stage.stories);
  const product: Product = { ...input.product, roles: stage.roster, stories: storiesAfter };
  const problems = [
    ...(await validateRoster(product, { root, overlays: stage.overlays, brief: stage.brief })),
    ...(stage.stories ? storiesProblems(product, stage.stories) : []),
  ];
  if (problems.length) return { plan: null, failures: problems };

  const before = await currentFiles(product.id, stage);
  const changes: FileChange[] = [];
  for (const path of [...new Set([...Object.keys(before), ...Object.keys(stage.files)])].sort()) {
    const a = before[path];
    const b = stage.files[path];
    if (a === b) continue;
    const status = a === undefined ? "new" : b === undefined ? "removed" : "changed";
    changes.push({ path, status, diff: status === "changed" ? await unifiedDiff(a!, b!) : "" });
  }
  const plan: ApplyPlan = { product, register: input.register, rosterBefore: input.register ? [] : input.product.roles, rosterAfter: stage.roster, changes, storiesBefore, storiesAfter };
  try {
    registryEdit(plan)(readFileSync(join(root, "agentsi.config.json"), "utf8"));
  } catch (err) {
    return { plan: null, failures: [(err as Error).message] };
  }
  return { plan, failures: [] };
}

export const storiesChanged = (plan: ApplyPlan): boolean =>
  JSON.stringify(plan.storiesBefore && normalStories(plan.storiesBefore)) !== JSON.stringify(plan.storiesAfter && normalStories(plan.storiesAfter));

const registryEdit =
  (plan: ApplyPlan) =>
  (text: string): string =>
    plan.register
      ? addProduct(text, plan.product)
      : updateProduct(text, plan.product.id, storiesChanged(plan) ? { roles: plan.rosterAfter, stories: plan.storiesAfter } : { roles: plan.rosterAfter });

/** Writes a plan: the brief and the overlays into the database, the roster into the registry; only the registry is committed. */
export async function applyPlan(plan: ApplyPlan, stageDir: string, root = dataRoot): Promise<CommitResult> {
  const { stage } = readStage(stageDir);
  if (!stage) throw new Error("Stage changed while applying.");
  const { id } = plan.product;
  await inTransaction(await getPool().connect(), async (db) => {
    for (const c of plan.changes) {
      if (c.path === "brief.json") {
        if (c.status === "removed") await deleteBrief(id, db);
        else await writeBrief(id, stage.brief, db);
        continue;
      }
      const agent = c.path.slice("agents/".length, -".md".length);
      if (c.status === "removed") await deleteOverlay(id, agent, db);
      else await writeOverlay(id, agent, stage.overlays[agent]!, db);
    }
    // Last, so a failed registry edit rolls the database back.
    editRegistryFile(join(root, "agentsi.config.json"), registryEdit(plan));
  });
  const what = `${plan.register ? "register and set roster" : "set roster"}${storiesChanged(plan) ? " and stories" : ""}`;
  return commitFiles(root, ["agentsi.config.json"], `roles(${id}): ${what}`);
}

export function registerableProduct(id: string, name: string | undefined, path: string | undefined): Product {
  if (!/^[a-z][a-z0-9-]*$/.test(id)) throw new Error(`Invalid product id "${id}".`);
  if (!name || !path) throw new Error("--register needs --name and --path.");
  if (!isAbsolute(path) || !existsSync(path)) throw new Error(`--path must be an existing absolute path, got "${path}".`);
  return { id, name, paths: [path], status: "active", roles: [] };
}
