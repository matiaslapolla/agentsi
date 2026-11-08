import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, join, relative, resolve } from "node:path";
import { configPath, dataRoot } from "./paths.ts";

export interface Product {
  id: string;
  name: string;
  paths: string[];
  status: "active" | "planned";
  /** Agent ids of the roster: a role (`dev`) or a specialist (`dev.backend`). */
  roles: string[];
  /** Story config: present only when Stories mode is on for the product. */
  stories?: StoryConfigBlock;
}

export interface StoryConfigBlock {
  /** Absolute path of the product repo; defaults to the first path. */
  repo?: string;
  /** Test command; `{files}` gets the repo-relative test paths. */
  test: string;
  gate: string[];
  /** Cap in USD for each story session call; defaults to `STORY_BUDGET_USD`. */
  budget_usd?: number;
}

export type StoryConfig = Required<StoryConfigBlock>;

export const STORY_BUDGET_USD = 5;

/** The block with its keys in registry order, so two blocks compare by value. */
export const normalStories = (s: StoryConfigBlock): StoryConfigBlock => ({
  ...(s.repo === undefined ? {} : { repo: s.repo }),
  test: s.test,
  gate: s.gate,
  ...(s.budget_usd === undefined ? {} : { budget_usd: s.budget_usd }),
});

/** Who approves the agency's work; the prompts name them and answer in their language. */
export interface Owner {
  name: string;
  /** Free text for the prompts, for example "English" or "Rioplatense Spanish". */
  language: string;
}

export const DEFAULT_OWNER: Owner = { name: "the owner", language: "English" };

export interface Registry {
  owner?: Partial<Owner>;
  products: Product[];
}

export interface Detection {
  product: Product;
  scope: string | null;
}

export function loadRegistry(file = configPath): Registry {
  return JSON.parse(readFileSync(file, "utf8")) as Registry;
}

/** The owner block with its defaults; throws on a block that is not an object or a key that is not a non-empty string. */
export function ownerOf(registry: Pick<Registry, "owner">): Owner {
  const block: unknown = registry.owner;
  if (block === undefined) return DEFAULT_OWNER;
  if (typeof block !== "object" || block === null || Array.isArray(block)) throw new Error("The owner block of agentsi.config.json must be an object.");
  const owner = { ...DEFAULT_OWNER };
  for (const key of ["name", "language"] as const) {
    const value = (block as Record<string, unknown>)[key];
    if (value === undefined) continue;
    if (typeof value !== "string" || !value.trim()) throw new Error(`owner.${key} in agentsi.config.json must be a non-empty string.`);
    owner[key] = value.trim();
  }
  return owner;
}

/** The owner of the registry in `root`; the defaults when `root` has no registry, as in a test root. */
export function loadOwner(root = dataRoot): Owner {
  const file = join(root, "agentsi.config.json");
  return existsSync(file) ? ownerOf(loadRegistry(file)) : DEFAULT_OWNER;
}

export function getProduct(id: string, registry = loadRegistry()): Product {
  const product = registry.products.find((p) => p.id === id);
  if (!product) throw new Error(`Unknown product "${id}"`);
  return product;
}

export function detectProduct(cwd: string, registry = loadRegistry()): Detection | null {
  const abs = resolve(cwd);
  let best: { product: Product; root: string } | null = null;
  for (const product of registry.products) {
    for (const root of product.paths) {
      const rel = relative(root, abs);
      const inside = rel === "" || (!rel.startsWith("..") && !isAbsolute(rel));
      if (inside && (!best || root.length > best.root.length)) best = { product, root };
    }
  }
  if (!best) return null;
  return { product: best.product, scope: null };
}

/** The resolved story config, or a throw when Stories mode is off for the product. */
export function storyConfig(product: Product): StoryConfig {
  const block = product.stories;
  if (!block) throw new Error(`Stories mode is off for ${product.id}: add a stories block to agentsi.config.json`);
  const repo = block.repo ?? product.paths[0];
  if (!repo) throw new Error(`Product ${product.id} has no path for its stories repo.`);
  if (typeof block.test !== "string" || !block.test.includes("{files}")) {
    throw new Error(`The stories block of ${product.id} needs a test command with {files}.`);
  }
  if (!Array.isArray(block.gate) || !block.gate.length || block.gate.some((g) => typeof g !== "string" || !g.trim())) {
    throw new Error(`The stories block of ${product.id} needs gate as a list of commands.`);
  }
  const budget = block.budget_usd === undefined ? STORY_BUDGET_USD : block.budget_usd;
  if (typeof budget !== "number" || !Number.isFinite(budget) || budget <= 0) {
    throw new Error(`The stories block of ${product.id} needs budget_usd as a positive number of USD, got ${JSON.stringify(block.budget_usd)}.`);
  }
  return { repo, test: block.test, gate: block.gate, budget_usd: budget };
}
