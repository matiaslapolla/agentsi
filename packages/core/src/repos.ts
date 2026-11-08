import { existsSync } from "node:fs";
import { basename, join } from "node:path";
import { getProduct, type Product } from "./registry.ts";

export interface RepoTarget {
  path: string;
  scope: string | null;
  /** Prefix that keeps refs unique when one product spans several repos. */
  prefix: string;
}

const isGitRepo = (dir: string) => existsSync(join(dir, ".git"));

export function repoTargets(product: Product | string, scope: string | null = null): RepoTarget[] {
  const p = typeof product === "string" ? getProduct(product) : product;
  const targets: RepoTarget[] = [];
  for (const root of p.paths) {
    if (isGitRepo(root)) targets.push({ path: root, scope: null, prefix: p.paths.length > 1 ? basename(root) : "" });
  }
  return scope ? targets.filter((t) => t.scope === scope) : targets;
}
