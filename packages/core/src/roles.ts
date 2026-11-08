import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { readOverlay as readStoredOverlay } from "./overlays.ts";
import { pluginAgentsDir, dataRoot } from "./paths.ts";

export type RoleSource = "global" | "overlay" | "custom";

export interface RoleDef {
  id: string;
  role: string;
  source: RoleSource;
  model: string;
  effort?: string;
  tools?: string[];
  body: string;
}

export interface RoleDirs {
  plugin: string;
  /** The overlay text of an agent in a product, or null when it has none. */
  overlay: (product: string, id: string) => Promise<string | null>;
}

/** Global roles under `plugin/agents` in `root`; overlays in the database. */
export const roleDirsAt = (root: string): RoleDirs => ({
  plugin: pluginAgentsDir(root),
  overlay: readStoredOverlay,
});

const defaultDirs = roleDirsAt(dataRoot);

/** A role, or `<role>.<specialty>`: lowercase words joined by single hyphens. Memory files use the same rule. */
export const AGENT_ID = /^[a-z]+(-[a-z]+)*(\.[a-z]+(-[a-z]+)*)?$/;
const ALLOWED_TOOLS = ["Read", "Grep", "Glob", "WebSearch"];

export function parseAgentFile(text: string): { frontmatter: Record<string, string>; body: string } {
  const m = /^---\r?\n([\s\S]*?)\r?\n---\r?\n?([\s\S]*)$/.exec(text);
  if (!m) return { frontmatter: {}, body: text.trim() };
  const frontmatter: Record<string, string> = {};
  for (const line of m[1]!.split(/\r?\n/)) {
    const kv = /^([A-Za-z][\w-]*):\s*(.*)$/.exec(line);
    if (kv) frontmatter[kv[1]!] = kv[2]!.trim();
  }
  return { frontmatter, body: m[2]!.trim() };
}

function assertAgentId(id: string): void {
  if (!AGENT_ID.test(id)) throw new Error(`Invalid agent id "${id}"`);
}

/** Keeps the built-in tools a meeting may grant; `warn` reports the rest. */
function parseTools(raw: string, warn: boolean, id: string): string[] {
  const names = raw.split(",").map((t) => t.trim()).filter(Boolean);
  const dropped = names.filter((t) => !ALLOWED_TOOLS.includes(t));
  if (warn && dropped.length) console.warn(`Agent "${id}": dropped tools not allowed in meetings: ${dropped.join(", ")}`);
  return [...new Set(names.filter((t) => ALLOWED_TOOLS.includes(t)))];
}

function readGlobal(role: string, dirs: RoleDirs): ReturnType<typeof parseAgentFile> {
  const file = join(dirs.plugin, `${role}.md`);
  if (!/^[a-z][a-z-]*$/.test(role) || !existsSync(file)) throw new Error(`Unknown role "${role}" (no ${file})`);
  return parseAgentFile(readFileSync(file, "utf8"));
}

async function readOverlay(id: string, product: string | undefined, dirs: RoleDirs): Promise<ReturnType<typeof parseAgentFile> | null> {
  if (!product) return null;
  const text = await dirs.overlay(product, id);
  return text === null ? null : parseAgentFile(text);
}

/** The role of an agent: its overlay's `extends`, else the part before the dot, else the id. */
export async function agentRole(id: string, product?: string, dirs: RoleDirs = defaultDirs): Promise<string> {
  const extended = (await readOverlay(id, product, dirs))?.frontmatter.extends;
  return extended || id.split(".")[0]!;
}

export async function loadRole(id: string, product?: string, dirs: RoleDirs = defaultDirs): Promise<RoleDef> {
  assertAgentId(id);
  const overlay = await readOverlay(id, product, dirs);
  const specialist = id.includes(".");

  if (!overlay) {
    if (specialist) throw new Error(`Specialist "${id}" has no overlay in ${product ?? "<product>"}`);
    const { frontmatter, body } = readGlobal(id, dirs);
    return toDef(id, id, "global", frontmatter, body, false);
  }

  const parent = overlay.frontmatter.extends;
  if (!parent) return toDef(id, id.split(".")[0]!, "custom", overlay.frontmatter, overlay.body, true);

  const base = readGlobal(parent, dirs);
  const merged = { ...base.frontmatter };
  for (const key of ["model", "effort", "tools"]) if (overlay.frontmatter[key]) merged[key] = overlay.frontmatter[key]!;
  const body = overlay.body ? `${base.body}\n\n## This product\n\n${overlay.body}` : base.body;
  return toDef(id, parent, "overlay", merged, body, Boolean(overlay.frontmatter.tools));
}

function toDef(id: string, role: string, source: RoleSource, fm: Record<string, string>, body: string, warnTools: boolean): RoleDef {
  return {
    id,
    role,
    source,
    model: fm.model || "sonnet",
    effort: fm.effort,
    tools: fm.tools ? parseTools(fm.tools, warnTools, id) : undefined,
    body,
  };
}
