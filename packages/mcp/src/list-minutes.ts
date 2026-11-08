import { archivedIds, type Queryable } from "@agentsi/core";

interface Row {
  id: string;
  question: string;
  status: string;
  created_at: Date;
  marks: Record<string, number>;
}

const pad = (n: number) => String(n).padStart(2, "0");
const stamp = (d: Date) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}`;

/** The `list_minutes` text. Archived minutes are left out unless the owner asks for them; then they carry an `archived` flag. */
export async function listMinutesText(db: Queryable, productId: string, limit: number, opts: { includeArchived?: boolean } = {}): Promise<string> {
  const archived = await archivedIds(productId, undefined, db);
  const { rows } = await db.query<Row>(
    `select m.id, m.question, m.status, m.created_at,
            coalesce((select jsonb_object_agg(k, n) from (
              select coalesce(p.mark, 'unmarked') as k, count(*) as n from priorities p where p.minutes_id = m.id group by 1) t), '{}') as marks
     from minutes m where m.product = $1 order by m.created_at desc limit $2`,
    [productId, opts.includeArchived ? limit : limit + archived.size],
  );
  const shown = (opts.includeArchived ? rows : rows.filter((m) => !archived.has(m.id))).slice(0, limit);
  if (!shown.length) return `No minutes yet for ${productId}.`;
  return shown
    .map((m) => {
      const marks = Object.entries(m.marks).map(([k, v]) => `${v} ${k}`).join(", ") || "no priorities";
      return `${m.id} | ${m.status} | ${stamp(m.created_at)} | ${m.question} | marks: ${marks}${archived.has(m.id) ? " | archived" : ""}`;
    })
    .join("\n");
}
