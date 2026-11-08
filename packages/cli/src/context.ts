import { detectProduct, isProductArchived, getProduct, migrateHint, pendingMigrations, type Product } from "@agentsi/core";
import { UsageError } from "./command.ts";

export interface ProductContext {
  product: Product;
  scope: string | null;
}

/** Resolves --product, or falls back to the product that owns the cwd. */
export function resolveProduct(flag: string | undefined, cwd = process.cwd()): ProductContext {
  const detected = detectProduct(cwd);
  if (flag) {
    const product = getProduct(flag);
    return { product, scope: detected?.product.id === product.id ? detected.scope : null };
  }
  if (!detected) throw new UsageError(`No product matches ${cwd}. Pass --product <id>.`);
  return detected;
}

/** Throws when the product is archived, and says how to bring it back. */
export async function assertNotArchived(product: Product): Promise<void> {
  if (await isProductArchived(product.id)) throw new Error(`Product ${product.id} is archived. Run agentsi restore --product ${product.id} to bring it back.`);
}

/** Prints `Run agentsi migrate` and returns true when a migration is pending, so a command can stop before a raw SQL error. An unreachable database prints one line and counts as blocked. */
export async function migrationPending(): Promise<boolean> {
  let pending: string[];
  try {
    pending = await pendingMigrations();
  } catch (err) {
    console.error(`Database unreachable: ${(err as Error).message || (err as { code?: string }).code}. Run pnpm db:up.`);
    return true;
  }
  if (pending.length) console.error(migrateHint(pending));
  return pending.length > 0;
}
