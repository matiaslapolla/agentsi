import { getPool, type Queryable } from "./db.ts";
import type { CommitResult } from "./exec.ts";
import { dataRoot } from "./paths.ts";
import { loadRegistry } from "./registry.ts";

const MINUTES_ID = /^[A-Za-z0-9_][A-Za-z0-9._-]*$/;

type Kind = "minutes" | "decision" | "story" | "product";

/** An archive change commits nothing: the record is a row of `archive_records`. */
const DONE: CommitResult = { committed: false };

/** The refs of one kind that a product has archived. A product that archived nothing has no row. */
async function refs(product: string, kind: Kind, db: Queryable = getPool()): Promise<string[]> {
  const { rows } = await db.query<{ ref: string }>("select ref from archive_records where product = $1 and kind = $2 order by ref", [product, kind]);
  return rows.map((r) => r.ref);
}

/** The ids of the archived minutes of one product. The data root argument is kept for old callers and ignored. */
export const archivedIds = async (product: string, _root = dataRoot, db?: Queryable): Promise<Set<string>> => new Set(await refs(product, "minutes", db));

/** The ids of the archived decisions of one product. An archived decision stays pending in the database. */
export const archivedDecisionIds = async (product: string, _root = dataRoot, db?: Queryable): Promise<Set<number>> =>
  new Set((await refs(product, "decision", db)).map(Number));

/** The ids of the archived stories of one product. */
export const archivedStoryIds = async (product: string, _root = dataRoot, db?: Queryable): Promise<Set<string>> => new Set(await refs(product, "story", db));

/** Flips one record. Throws when it is already in the wanted state; the insert and the delete are atomic, so two writers cannot both win. */
async function flip(product: string, kind: Kind, ref: string, archive: boolean, label: string): Promise<CommitResult> {
  const db = getPool();
  const { rows } = archive
    ? await db.query("insert into archive_records (product, kind, ref) values ($1, $2, $3) on conflict do nothing returning ref", [product, kind, ref])
    : await db.query("delete from archive_records where product = $1 and kind = $2 and ref = $3 returning ref", [product, kind, ref]);
  if (!rows.length) throw new Error(`${label} is ${archive ? "already archived" : "not archived"}.`);
  return DONE;
}

async function change(product: string, id: string, archive: boolean): Promise<CommitResult> {
  if (!MINUTES_ID.test(id) || id.includes("..")) throw new Error(`Bad minutes id "${id}".`);
  const { rows } = await getPool().query("select 1 from minutes where id = $1 and product = $2 and data is not null", [id, product]);
  if (!rows.length) throw new Error(`No minutes ${id} for ${product}.`);
  return flip(product, "minutes", id, archive, `Minutes ${id}`);
}

/** Records the minutes in the archive record. The minutes row is not touched. */
export const archiveMinutes = (product: string, id: string, _root = dataRoot) => change(product, id, true);

/** Takes the minutes out of the archive record. */
export const restoreMinutes = (product: string, id: string, _root = dataRoot) => change(product, id, false);

function changeDecision(product: string, id: number, archive: boolean): Promise<CommitResult> {
  if (!Number.isInteger(id) || id < 1) throw new Error(`Bad decision id "${id}".`);
  return flip(product, "decision", String(id), archive, `Decision ${id}`);
}

/** Records the decision in the archive record. It stays pending in the database; the readers leave it out. */
export const archiveDecision = (product: string, id: number, _root = dataRoot) => changeDecision(product, id, true);

/** Takes the decision out of the archive record. */
export const restoreDecision = (product: string, id: number, _root = dataRoot) => changeDecision(product, id, false);

/** Flips the story in the archive record; `guard` runs first and may throw. */
export async function changeStoryArchive(product: string, id: string, archive: boolean, _root: string, guard: () => void | Promise<void>): Promise<CommitResult> {
  await guard();
  return flip(product, "story", id, archive, `Story ${id}`);
}

/** True when the whole product is archived. */
export async function isProductArchived(product: string, _root = dataRoot): Promise<boolean> {
  return (await refs(product, "product")).length > 0;
}

/** Marks the whole product as archived. The registry entry and the data stay. */
export const archiveProduct = (product: string, _root = dataRoot) => flip(product, "product", "", true, `Product ${product}`);

/** Clears the archived mark of the product. */
export const restoreProduct = (product: string, _root = dataRoot) => flip(product, "product", "", false, `Product ${product}`);

/** The product that archived the decision, or null. Looks through every product of the registry. */
export async function archivedDecisionProduct(id: number, _root = dataRoot): Promise<string | null> {
  try {
    for (const p of loadRegistry().products) if ((await archivedDecisionIds(p.id)).has(id)) return p.id;
  } catch {
    // no registry or no database: nothing is archived
  }
  return null;
}
