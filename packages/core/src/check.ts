import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve } from "node:path";
import { readBrief, resolveUnderRoots, type Brief } from "./brief.ts";
import { git, run } from "./exec.ts";
import { repoTargets } from "./repos.ts";
import { dataRoot } from "./paths.ts";
import { getProduct, type Product } from "./registry.ts";
import { agentRole } from "./roles.ts";
import { listStories, type Story } from "./story.ts";
import type { Minutes, Source } from "./types.ts";

export interface CheckResult {
  passed: boolean;
  failures: string[];
}

export interface CheckOptions {
  /** Agent ids of the roster. Each of their roles must have at least one answering agent. Defaults to the product's roster. */
  roles?: string[];
  /** Overrides the registry lookup (tests, ad-hoc products). */
  product?: Product;
  /** The product's brief. Omitted: read from the workspace. `null`: none, today's rules apply. */
  brief?: Brief | null;
  /** The AgentSI checkout whose workspace holds the stories. */
  root?: string;
}

const NORTH_STAR = /^(CLAUDE|CONTEXT|PRODUCT|DESIGN|README)\.md$/;

function projectRoots(product: Product): { roots: string[]; repos: string[] } {
  const roots = product.paths.filter((p) => existsSync(p));
  const repos = [...new Set([...repoTargets(product).map((t) => t.path), ...roots])];
  return { roots, repos };
}

async function resolveCommit(ref: string, repos: string[]): Promise<boolean> {
  if (!/^[0-9a-f]{4,40}$/i.test(ref)) return false;
  for (const repo of repos) if ((await git(repo, "cat-file", "-e", `${ref}^{commit}`)).ok) return true;
  return false;
}

async function resolveIssue(ref: string, product: Product, repos: string[]): Promise<boolean> {
  const m = /^(?:(.+?)[#/])?#?(\d+)$/.exec(ref.trim());
  if (!m) return false;
  const candidates = m[1] ? repoTargets(product).filter((t) => t.prefix === m[1]).map((t) => t.path) : repos;
  for (const repo of candidates) {
    if ((await run("gh", ["issue", "view", m[2]!, "--json", "number"], repo, 25_000)).ok) return true;
  }
  return false;
}

function resolvePath(rawRef: string, roots: string[]): string | null {
  const ref = rawRef.replace(/(#L\d+(-L?\d+)?|:\d+(-\d+)?)$/, "");
  if (isAbsolute(ref)) {
    return existsSync(ref) && roots.some((r) => !relative(r, ref).startsWith("..")) ? ref : null;
  }
  for (const root of roots) {
    const full = resolve(join(root, ref));
    if (!relative(root, full).startsWith("..") && existsSync(full)) return full;
  }
  return null;
}

function isNorthStar(fullPath: string, roots: string[]): boolean {
  for (const root of roots) {
    const rel = relative(root, fullPath);
    if (rel.startsWith("..")) continue;
    const parts = rel.split("/");
    if (parts.length === 1 && NORTH_STAR.test(parts[0]!)) return true;
    if (parts[0] === "docs" && parts.length > 1) return true;
  }
  return false;
}

const underAny = (full: string, declared: string[]): boolean => declared.some((d) => !relative(d, full).startsWith(".."));

function isHttpUrl(ref: string): boolean {
  try {
    const u = new URL(ref);
    return u.protocol === "http:" || u.protocol === "https:";
  } catch {
    return false;
  }
}

export interface SourceContext {
  product: Product;
  roots: string[];
  repos: string[];
  /** Absolute paths declared as north stars by the brief. When set, only these count. */
  northStars?: string[];
  stories: Pick<Story, "id" | "title">[];
}

export async function sourceContext(product: Product, root = dataRoot): Promise<SourceContext> {
  // A database that cannot answer leaves no story to resolve, as a product with no stories does.
  const stories = await listStories(product.id, root).catch(() => []);
  return { product, ...projectRoots(product), stories };
}

async function resolveSourceDetailed(s: Source, ctx: SourceContext): Promise<{ ok: boolean; northStar: boolean }> {
  if (s.type === "commit") return { ok: await resolveCommit(s.ref, ctx.repos), northStar: false };
  if (s.type === "issue") return { ok: await resolveIssue(s.ref, ctx.product, ctx.repos), northStar: false };
  if (s.type === "url") return { ok: isHttpUrl(s.ref), northStar: false };
  if (s.type === "story") return { ok: ctx.stories.some((st) => st.id === s.ref), northStar: false };
  if (s.type === "path") {
    const full = resolvePath(s.ref, ctx.roots);
    const northStar = full !== null && (ctx.northStars ? underAny(full, ctx.northStars) : isNorthStar(full, ctx.roots));
    return { ok: full !== null, northStar };
  }
  return { ok: false, northStar: false };
}

export async function resolveSource(s: Source, product: Product, ctx?: SourceContext): Promise<boolean> {
  return (await resolveSourceDetailed(s, ctx ?? (await sourceContext(product)))).ok;
}

export async function check(m: Minutes, opts: CheckOptions = {}): Promise<CheckResult> {
  const product = opts.product ?? getProduct(m.product);
  const brief = opts.brief === undefined ? await readBrief(m.product) : opts.brief;
  const ctx = await sourceContext(product, opts.root);
  if (brief) ctx.northStars = brief.north_stars.map((p) => resolveUnderRoots(p, product.paths)).filter((p): p is string => p !== null);
  const failures: string[] = [];
  let northStar = false;

  if (m.priorities.length === 0) failures.push("No priorities.");

  for (const [i, p] of m.priorities.entries()) {
    if (brief && !brief.objectives.some((o) => o.id === p.objective)) {
      failures.push(
        `Priority #${i} "${p.title}" objective ${p.objective ? `"${p.objective}"` : "is missing and"} must be one of the brief ids: ${brief.objectives.map((o) => o.id).join(", ")}.`,
      );
    }
    if (!p.sources?.length) {
      failures.push(`Priority #${i} "${p.title}" has no source.`);
      continue;
    }
    for (const s of p.sources as Source[]) {
      const res = await resolveSourceDetailed(s, ctx);
      if (res.northStar) northStar = true;
      if (!res.ok) failures.push(`Priority #${i}: ${s.type} source "${s.ref}" does not resolve.`);
    }
  }

  for (const [i, d] of m.decisions.entries()) {
    for (const idx of d.priorities ?? []) {
      if (!Number.isInteger(idx) || idx < 0 || idx >= m.priorities.length) {
        failures.push(`Decision #${i} "${d.text}" links priority index ${idx}, which does not exist (0 to ${m.priorities.length - 1}).`);
      }
    }
  }

  if (!northStar && brief?.north_stars.length) {
    failures.push(`No priority cites a north star declared in the brief (${brief.north_stars.join(", ")}).`);
  } else if (!northStar && !brief) {
    failures.push("No priority cites a north-star or guideline doc (CLAUDE/CONTEXT/PRODUCT/DESIGN/README.md or docs/**).");
  }

  const byRole = new Map<string, string[]>();
  for (const id of opts.roles ?? product.roles) {
    // A database that cannot answer leaves no overlay, so the role is the part of the id before the dot.
    const role = await agentRole(id, product.id).catch(() => id.split(".")[0]!);
    byRole.set(role, [...(byRole.get(role) ?? []), id]);
  }
  for (const [role, agents] of byRole) {
    if (agents.some((id) => m.roles?.[id]?.answer?.trim())) continue;
    const named = agents.length === 1 && agents[0] === role ? "" : ` (agents: ${agents.join(", ")})`;
    failures.push(`Role "${role}" has no answer${named}.`);
  }

  if (!m.script?.some((s) => s.text?.trim())) failures.push("Call script is empty.");
  for (const st of ctx.stories) {
    if (m.script?.some((s) => new RegExp(`\\b${st.id}\\b`).test(s.text ?? ""))) {
      failures.push(`Call script names story ${st.id} by its id: say its title "${st.title}".`);
    }
  }

  return { passed: failures.length === 0, failures };
}

export function readMinutesFile(file: string): Minutes {
  return JSON.parse(readFileSync(file, "utf8")) as Minutes;
}
