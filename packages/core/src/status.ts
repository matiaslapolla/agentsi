import { archivedDecisionIds, archivedIds } from "./archive.ts";
import { getPool, type Queryable } from "./db.ts";
import { dataRoot } from "./paths.ts";
import { getProduct } from "./registry.ts";

const ago = (d: Date | null): string => {
  if (!d) return "never";
  const min = Math.round((Date.now() - d.getTime()) / 60_000);
  if (min < 60) return `${min} min ago`;
  if (min < 48 * 60) return `${Math.round(min / 60)} h ago`;
  return `${Math.round(min / 1440)} d ago`;
};

export async function statusMarkdown(productId: string, pool: Queryable = getPool(), root = dataRoot): Promise<string> {
  const product = getProduct(productId);
  const archived = await archivedIds(productId, root);
  const [counts, lastIndexed, lastCommit, pending, recent, totalMinutes] = await Promise.all([
    pool.query<{ kind: string; n: string }>(
      "select kind, count(*) as n from documents where product = $1 group by kind",
      [productId],
    ),
    pool.query<{ at: Date | null }>("select max(indexed_at) as at from index_state where product = $1", [productId]),
    pool.query<{ title: string; updated_at: Date }>(
      "select title, updated_at from documents where product = $1 and kind = 'commit' order by updated_at desc limit 1",
      [productId],
    ),
    pool.query<{ id: string; text: string; minutes_id: string | null }>(
      "select id, text, minutes_id from decisions where product = $1 and status = 'pending' order by id",
      [productId],
    ),
    pool.query<{ id: string; question: string; status: string; marks: Record<string, string> }>(
      `select m.id, m.question, m.status,
              coalesce((select jsonb_object_agg(k, n) from (
                select coalesce(p.mark, 'unmarked') as k, count(*) as n from priorities p where p.minutes_id = m.id group by 1) t), '{}') as marks
       from minutes m where m.product = $1 order by m.created_at desc limit $2`,
      [productId, 3 + archived.size],
    ),
    pool.query<{ id: string }>("select id from minutes where product = $1", [productId]),
  ]);
  const archivedDecisions = await archivedDecisionIds(productId, root);
  pending.rows = pending.rows.filter((d) => !archivedDecisions.has(Number(d.id)));
  const n = Object.fromEntries(counts.rows.map((r) => [r.kind, Number(r.n)]));
  const last = lastCommit.rows[0];

  const out: string[] = [`# ${product.name} — AgentSI status`, ""];
  out.push(`- Roles: ${product.roles.join(", ")} · status ${product.status}`);
  out.push(
    `- Index: ${n.doc ?? 0} docs, ${n.commit ?? 0} commits, ${n.issue ?? 0} open issues, indexed ${ago(lastIndexed.rows[0]?.at ?? null)}`,
  );
  out.push(`- Last commit: ${last ? `${last.updated_at.toISOString().slice(0, 10)} ${last.title}` : "none indexed"}`);
  out.push(`- Pending decisions: ${pending.rows.length}`);
  out.push(`- Minutes: ${totalMinutes.rows.filter((r) => !archived.has(r.id)).length} total`, "");

  out.push("## Pending decisions", "");
  if (!pending.rows.length) out.push("_None._");
  for (const d of pending.rows) out.push(`- [${d.id}] ${d.text}${d.minutes_id ? ` (${d.minutes_id})` : ""}`);
  out.push("");

  out.push("## Last minutes", "");
  const shown = recent.rows.filter((r) => !archived.has(r.id)).slice(0, 3);
  if (!shown.length) out.push("_No meetings yet._");
  for (const m of shown) {
    const marks = Object.entries(m.marks).map(([k, v]) => `${v} ${k}`).join(", ") || "no priorities";
    out.push(`- ${m.id} — ${m.question} — ${m.status} — ${marks}`);
  }
  return out.join("\n") + "\n";
}
