import { cache } from "react";
import { getPool } from "@agentsi/core";
import { listStories } from "@agentsi/core/story";
import { ROOT } from "./env.ts";
import { rows } from "./db.ts";
import { computeInbox, type DecisionStatus, type InboxMinutes } from "./inbox.ts";
import { readRegistry } from "./registry.ts";
import { archivedRefs, audioFile, listMinutes, safeSegment } from "./workspace.ts";

interface DecisionDb {
  id: string | number;
  product: string;
  minutes_id: string | null;
  status: DecisionStatus;
  decided_at: Date | string | null;
}

async function heardCalls(product: string): Promise<Set<string>> {
  const found = await rows<{ minutes_id: string }>("select minutes_id from listens where product = $1", [product]);
  return new Set(found.map((r) => String(r.minutes_id)));
}

/** The one signal that needs storage: a call played to the end, one row per meeting. False when the call is not there or is already stored. */
export async function recordListen(product: string, minutesId: string): Promise<boolean> {
  const p = safeSegment(product);
  const id = safeSegment(minutesId);
  if (!p || !id || !audioFile(p, id)) return false;
  const stored = await getPool().query(
    "insert into listens (product, minutes_id, at) values ($1, $2, $3) on conflict (product, minutes_id) do nothing returning minutes_id",
    [p, id, new Date().toISOString()],
  );
  return stored.rows.length > 0;
}

/** Undoes a listen: drops only that meeting's row and keeps the others as they are. */
export async function removeListenRecord(product: string, minutesId: string): Promise<boolean> {
  const p = safeSegment(product);
  const id = safeSegment(minutesId);
  if (!p || !id) return false;
  const removed = await getPool().query("delete from listens where product = $1 and minutes_id = $2 returning minutes_id", [p, id]);
  return removed.rows.length > 0;
}

export async function storyCount(product: string, root = ROOT): Promise<number> {
  try {
    return (await listStories(product, root)).length;
  } catch {
    return 0;
  }
}

const inbox = cache(async (includeArchived: boolean) => {
  const products = readRegistry();
  const [lists, decisions] = await Promise.all([
    Promise.all(products.map((p) => listMinutes(p.id))),
    rows<DecisionDb>("select id, product, minutes_id, status, decided_at from decisions order by id"),
  ]);
  const archived = new Map(await Promise.all(products.map(async (p) => [p.id, await archivedRefs(p.id, "decision")] as const)));
  const heardOf = new Map(await Promise.all(products.map(async (p) => [p.id, await heardCalls(p.id)] as const)));
  const byMinutes = new Map<string, DecisionDb[]>();
  for (const d of decisions) {
    if (!d.minutes_id) continue;
    if (!includeArchived && archived.get(d.product)?.has(String(d.id))) continue;
    byMinutes.set(d.minutes_id, [...(byMinutes.get(d.minutes_id) ?? []), d]);
  }
  const minutes: InboxMinutes[] = lists.flatMap((list, i) => {
    const product = products[i].id;
    const heard = heardOf.get(product) ?? new Set<string>();
    return list.map((m) => {
      const db = byMinutes.get(m.id);
      return {
        id: m.id,
        product,
        question: m.question,
        createdAt: m.created_at,
        valid: m.status === "valid",
        priorities: (m.priorities ?? []).map((p) => ({ mark: p.mark ?? null, isNew: !!p.is_new, owner: p.owner_role })),
        decisions: db
          ? db.map((d) => ({ status: d.status, decidedAt: d.decided_at ? new Date(d.decided_at).toISOString() : null }))
          : (m.decisions ?? []).map((d) => ({ status: d.status ?? "pending", decidedAt: d.decided_at ?? null })),
        hasCall: !!audioFile(product, m.id),
        heard: heard.has(m.id),
      };
    });
  });
  const state = computeInbox({ products, minutes, now: new Date() });
  // Archived stories are never counted on the card.
  return { products: await Promise.all(state.products.map(async (p) => ({ ...p, stories: await storyCount(p.id) }))), minutes };
});

/** The inbox state. Archived decisions are left out of the counts unless `includeArchived` is set. */
export const loadInbox = (opts: { includeArchived?: boolean } = {}) => inbox(opts.includeArchived ?? false);
