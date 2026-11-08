import { existsSync } from "node:fs";
import { join } from "node:path";
import { readBrief, validateBrief, type Brief } from "./brief.ts";
import { readOverlay, readOverlays } from "./overlays.ts";
import { dataRoot, pluginAgentsDir } from "./paths.ts";
import type { Product } from "./registry.ts";
import { AGENT_ID, loadRole, parseAgentFile, type RoleDef, type RoleDirs } from "./roles.ts";

export { deleteOverlay, readOverlay, readOverlays, writeOverlay } from "./overlays.ts";

/** Where a roster lives: the repo root, and optionally a stage that stands in for the product's stored overlays and brief. */
export interface RosterSource {
  root?: string;
  /** Overlay text by agent id, instead of the overlays in the database. */
  overlays?: Record<string, string>;
  /** A brief instead of the one in the database; null for none. */
  brief?: Brief | null;
}

export interface RosterEntry {
  id: string;
  role: string | null;
  source: RoleDef["source"] | null;
  model: string | null;
  effort: string | null;
  error: string | null;
}

const rootOf = (src: RosterSource) => src.root ?? dataRoot;

function roleDirs(src: RosterSource = {}): RoleDirs {
  const { overlays } = src;
  return { plugin: pluginAgentsDir(rootOf(src)), overlay: overlays ? async (_product, id) => overlays[id] ?? null : readOverlay };
}

export function loadAgent(id: string, product: string, src: RosterSource = {}): Promise<RoleDef> {
  return loadRole(id, product, roleDirs(src));
}

/** One entry per roster agent; an agent that does not resolve carries its error instead of a definition. */
export async function describeRoster(product: Product, src: RosterSource = {}): Promise<RosterEntry[]> {
  const entries: RosterEntry[] = [];
  for (const id of product.roles) {
    try {
      const def = await loadAgent(id, product.id, src);
      entries.push({ id, role: def.role, source: def.source, model: def.model, effort: def.effort ?? null, error: null });
    } catch (err) {
      entries.push({ id, role: null, source: null, model: null, effort: null, error: (err as Error).message });
    }
  }
  return entries;
}

/** Failures of a roster, its overlays and its brief; empty when everything holds. */
export async function validateRoster(product: Product, src: RosterSource = {}): Promise<string[]> {
  const failures: string[] = [];
  const roster = product.roles;
  if (roster.length === 0) failures.push("The roster is empty.");

  const seen = new Set<string>();
  for (const id of roster) {
    if (seen.has(id)) failures.push(`Agent "${id}" is repeated in the roster.`);
    seen.add(id);
    if (!AGENT_ID.test(id)) failures.push(`Agent id "${id}" does not match ${AGENT_ID}.`);
  }
  for (const e of await describeRoster(product, src)) if (e.error && AGENT_ID.test(e.id)) failures.push(`Agent "${e.id}": ${e.error}`);

  const plugin = roleDirs(src).plugin;
  const overlays = src.overlays ?? (await readOverlays(product.id));
  for (const id of Object.keys(overlays).sort()) {
    const file = `${id}.md`;
    if (!AGENT_ID.test(id)) {
      failures.push(`Overlay "${file}" is not named after a valid agent id.`);
      continue;
    }
    if (roster.includes(id)) continue;
    failures.push(`Overlay "${file}" belongs to an agent that is not in the roster.`);
    const parent = parseAgentFile(overlays[id]!).frontmatter.extends;
    if (parent && !existsSync(join(plugin, `${parent}.md`))) failures.push(`Overlay "${file}" extends "${parent}", which is not a global role.`);
  }

  const brief = src.brief === undefined ? await readBrief(product.id) : src.brief;
  if (brief) failures.push(...validateBrief(brief, product, roster).map((f) => `Brief: ${f}`));
  return failures;
}
