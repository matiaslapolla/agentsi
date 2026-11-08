import { getPool, type Queryable } from "./db.ts";
import type { Minutes, UsageRow } from "./types.ts";

const parse = <T>(v: unknown): T => (typeof v === "string" ? JSON.parse(v) : v) as T;

/** Every saved minutes of the product, oldest first. Rows that predate the `data` column are not listed until `agentsi data import` fills them. */
export async function readMinutes(product: string, db: Queryable = getPool()): Promise<Minutes[]> {
  const { rows } = await db.query<{ data: unknown }>("select data from minutes where product = $1 and data is not null order by id", [product]);
  return rows.map((r) => parse<Minutes>(r.data));
}

/** The minutes with this id, or null. Ids are global: the minutes table keys on the id alone. */
export async function findMinutes(id: string, db: Queryable = getPool()): Promise<Minutes | null> {
  const { rows } = await db.query<{ data: unknown }>("select data from minutes where id = $1 and data is not null", [id]);
  return rows[0] ? parse<Minutes>(rows[0].data) : null;
}

/** The per-role usage rows stored with the minutes; empty when there are none. */
export async function readUsage(id: string, db: Queryable = getPool()): Promise<UsageRow[]> {
  const { rows } = await db.query<{ usage: unknown }>("select usage from minutes where id = $1", [id]);
  const usage = rows[0]?.usage == null ? [] : parse<UsageRow[]>(rows[0].usage);
  return Array.isArray(usage) ? usage : [];
}

/** Stores the minutes JSON as it is now, keeping every other column. */
export async function saveMinutesData(m: Minutes, db: Queryable = getPool()): Promise<void> {
  await db.query("update minutes set data = $2::jsonb where id = $1", [m.id, JSON.stringify(m)]);
}
