import { archivedDecisionIds } from "./archive.ts";
import { getPool } from "./db.ts";
import { readMinutes } from "./minutes-store.ts";
import type { DecisionStore } from "./reflection-decision.ts";
import type { Minutes, Source } from "./types.ts";

export type PendingItem =
  | { kind: "priority"; minutes_id: string; idx: number; title: string; why: string; owner_role: string; objective: string | null; sources: Source[] }
  | { kind: "decision"; id: number; minutes_id: string | null; text: string; why: string | null; action: string | null; priorities?: number[] };

export interface Pending {
  items: PendingItem[];
  /** Set when the database could not be read, so the list holds what was read before the failure (nothing, when the minutes failed). */
  decisions_error?: string;
}

interface DecisionRow {
  id: string;
  minutes_id: string | null;
  text: string;
  why: string | null;
  action: string | null;
}

/** Unmarked priorities and pending decisions, oldest minutes first. Product decisions come last. */
export async function pendingItems(product: string, o: { root?: string; db?: DecisionStore } = {}): Promise<Pending> {
  let minutes: Minutes[] = [];
  let rows: DecisionRow[] = [];
  let decisions_error: string | undefined;
  try {
    minutes = (await readMinutes(product)).filter((m) => m.status === "valid");
    const archived = await archivedDecisionIds(product, o.root);
    rows = (
      await (o.db ?? getPool()).query<DecisionRow>(
        "select id, minutes_id, text, why, action from decisions where product = $1 and status = 'pending' order by id",
        [product],
      )
    ).rows.filter((r) => !archived.has(Number(r.id)));
  } catch (err) {
    decisions_error = (err as Error).message || (err as { code?: string }).code || "unknown error";
  }
  /** The unmarked priorities the decision links in its minutes. Old minutes and product decisions link none. */
  const linked = (r: DecisionRow): number[] => {
    const m = minutes.find((x) => x.id === r.minutes_id);
    const d = m?.decisions.find((x) => x.text === r.text);
    if (!m || !d?.priorities) return [];
    return [...new Set(d.priorities)].filter((i) => Number.isInteger(i) && m.priorities[i] !== undefined && !m.priorities[i]!.mark);
  };
  const decisionItem = (r: DecisionRow): PendingItem => ({
    kind: "decision", id: Number(r.id), minutes_id: r.minutes_id, text: r.text, why: r.why, action: r.action, priorities: linked(r),
  });
  const items: PendingItem[] = [];
  const used = new Set<string>();
  for (const m of minutes) {
    m.priorities.forEach((p, idx) => {
      if (p.mark) return;
      items.push({ kind: "priority", minutes_id: m.id, idx, title: p.title, why: p.why, owner_role: p.owner_role, objective: p.objective ?? null, sources: p.sources });
    });
    for (const r of rows) {
      if (r.minutes_id !== m.id) continue;
      used.add(r.id);
      items.push(decisionItem(r));
    }
  }
  for (const r of rows) if (!used.has(r.id)) items.push(decisionItem(r));
  return decisions_error ? { items, decisions_error } : { items };
}
