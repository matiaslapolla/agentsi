import { getPool, type Queryable } from "./db.ts";

/** The text of the overlay of one agent in a product (frontmatter and body, as the file had it), or null. */
export async function readOverlay(product: string, agent: string): Promise<string | null> {
  const { rows } = await getPool().query<{ body: string }>("select body from agent_overlays where product = $1 and agent = $2", [product, agent]);
  return rows[0]?.body ?? null;
}

/** Every overlay of a product by agent id. */
export async function readOverlays(product: string): Promise<Record<string, string>> {
  const { rows } = await getPool().query<{ agent: string; body: string }>("select agent, body from agent_overlays where product = $1 order by agent", [product]);
  return Object.fromEntries(rows.map((r) => [r.agent, r.body]));
}

/** Stores the overlay of one agent, replacing the one it had. */
export async function writeOverlay(product: string, agent: string, text: string, db: Queryable = getPool()): Promise<void> {
  await db.query(
    `insert into agent_overlays (product, agent, body) values ($1, $2, $3)
     on conflict (product, agent) do update set body = excluded.body`,
    [product, agent, text],
  );
}

/** Removes the overlay of one agent; true when it had one. */
export async function deleteOverlay(product: string, agent: string, db: Queryable = getPool()): Promise<boolean> {
  const { rows } = await db.query("delete from agent_overlays where product = $1 and agent = $2 returning agent", [product, agent]);
  return rows.length > 0;
}
