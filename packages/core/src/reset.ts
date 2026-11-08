import { readdirSync } from "node:fs";
import { join } from "node:path";
import { deleteBrief, readBrief, writeBrief, type Brief } from "./brief.ts";
import { getPool, inTransaction } from "./db.ts";
import { deleteMemory, memoryAgents } from "./memory.ts";
import { commitFiles, type CommitResult } from "./exec.ts";
import { deleteOverlay, readOverlays } from "./overlays.ts";
import { pluginAgentsDir, dataRoot } from "./paths.ts";
import type { Product } from "./registry.ts";
import { editRegistryFile, updateProduct } from "./registry-edit.ts";

/** Order of the global roster; any other global role follows alphabetically. */
const GLOBAL_ORDER = ["pm", "social", "dev", "qa", "scout"];

export interface ResetPlan {
  product: Product;
  /** `null` resets the whole product. */
  agent: string | null;
  /** What to delete from the database: `brief.json`, `agents/<id>.md` and `memory/<id>.json`. */
  removals: string[];
  /** The brief to store instead of the current one, under `brief.json`, as JSON text. */
  writes: Record<string, string>;
  rosterBefore: string[];
  rosterAfter: string[];
}

export function globalRoles(root = dataRoot): string[] {
  const dir = pluginAgentsDir(root);
  const names = readdirSync(dir).filter((f) => f.endsWith(".md") && !f.startsWith("_")).map((f) => f.slice(0, -3));
  const rank = (n: string) => (GLOBAL_ORDER.includes(n) ? GLOBAL_ORDER.indexOf(n) : GLOBAL_ORDER.length);
  return names.sort((a, b) => rank(a) - rank(b) || a.localeCompare(b));
}

/**
 * What reset would do. `--all` keeps the global roles of the roster and drops specialists and custom roles
 * (the five global roles when none is left). An agent that has no global role to fall back to (a specialist or a custom role)
 * leaves the roster, and its entry leaves the brief; a global role only loses its overlay.
 */
export async function planReset(o: { product: Product; agent: string | null; memory: boolean; root?: string }): Promise<ResetPlan> {
  const root = o.root ?? dataRoot;
  const { product, agent } = o;
  const rosterBefore = product.roles;
  const brief = await readBrief(product.id);
  const overlays = Object.keys(await readOverlays(product.id)).map((id) => `${id}.md`).sort();
  const memories = (await memoryAgents(product.id)).map((id) => `${id}.json`).sort();

  if (agent === null) {
    const removals = [
      ...(brief ? ["brief.json"] : []),
      ...overlays.map((f) => `agents/${f}`),
      ...(o.memory ? memories.map((f) => `memory/${f}`) : []),
    ];
    const globals = globalRoles(root);
    const kept = rosterBefore.filter((r) => globals.includes(r));
    return { product, agent, removals, writes: {}, rosterBefore, rosterAfter: kept.length ? kept : globals };
  }

  const overlay = `agents/${agent}.md`;
  const memory = `memory/${agent}.json`;
  const removals = [...(overlays.includes(`${agent}.md`) ? [overlay] : []), ...(o.memory && memories.includes(`${agent}.json`) ? [memory] : [])];
  const inRoster = rosterBefore.includes(agent);
  if (!inRoster && !removals.length) throw new Error(`Agent "${agent}" is not in the roster of ${product.id} and has no data to reset.`);

  const hasGlobal = !agent.includes(".") && globalRoles(root).includes(agent);
  const leaves = inRoster && !hasGlobal;
  const rosterAfter = leaves ? rosterBefore.filter((r) => r !== agent) : rosterBefore;
  if (leaves && rosterAfter.length === 0) throw new Error(`Resetting "${agent}" would leave ${product.id} with an empty roster.`);

  const writes: Record<string, string> = {};
  if (leaves && brief?.roles && agent in brief.roles) {
    const roles = { ...brief.roles };
    delete roles[agent];
    writes["brief.json"] = JSON.stringify({ ...brief, roles }, null, 2) + "\n";
  }
  return { product, agent, removals, writes, rosterBefore, rosterAfter };
}

export const isNoop = (plan: ResetPlan): boolean =>
  !plan.removals.length && !Object.keys(plan.writes).length && plan.rosterBefore.join() === plan.rosterAfter.join();

/** Changes the database and the registry; only the registry is committed. */
export async function applyReset(plan: ResetPlan, root = dataRoot): Promise<CommitResult> {
  const id = plan.product.id;
  await inTransaction(await getPool().connect(), async (db) => {
    for (const rel of plan.removals) {
      if (rel === "brief.json") await deleteBrief(id, db);
      else if (rel.startsWith("agents/")) await deleteOverlay(id, rel.slice("agents/".length, -".md".length), db);
      else await deleteMemory(id, rel.slice("memory/".length, -".json".length), db);
    }
    if (plan.writes["brief.json"] !== undefined) await writeBrief(id, JSON.parse(plan.writes["brief.json"]) as Brief, db);
    if (plan.rosterBefore.join() !== plan.rosterAfter.join()) {
      editRegistryFile(join(root, "agentsi.config.json"), (text) => updateProduct(text, id, { roles: plan.rosterAfter }));
    }
  });
  return commitFiles(root, ["agentsi.config.json"], `reset(${id}): ${plan.agent ?? "all"}`);
}
