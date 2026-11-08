import { archivedDecisionIds, isMissingSchemaError, migrateHint, pendingMigrations, type Queryable } from "@agentsi/core";

interface Row {
  id: string;
  minutes_id: string | null;
  text: string;
  why: string | null;
  action: string | null;
  total: string;
}

/** Pending decisions of the product, without the archived ones. `root` is the data root that holds the archive record. */
export async function pendingDecisionsText(db: Queryable, productId: string, limit: number, root?: string): Promise<string> {
  let rows: Row[];
  let archived: Set<number>;
  try {
    archived = await archivedDecisionIds(productId, root, db);
    ({ rows } = await db.query<Row>(
      `select id, minutes_id, text, why, action, count(*) over () as total from decisions
       where product = $1 and status = 'pending' and id <> all($3::bigint[]) order by minutes_id nulls last, id limit $2`,
      [productId, limit, [...archived]],
    ));
  } catch (err) {
    if (!isMissingSchemaError(err)) throw err;
    return migrateHint(await pendingMigrations(db));
  }
  // The query already leaves the archived ids out; this keeps the list right for a reader that ignores the filter.
  const kept = rows.filter((d) => !archived.has(Number(d.id)));
  if (!kept.length) return `No pending decisions for ${productId}.`;
  const lines: string[] = [];
  let group: string | null | undefined;
  for (const d of kept) {
    if (d.minutes_id !== group) lines.push(`minutes ${d.minutes_id ?? "-"}:`);
    group = d.minutes_id;
    lines.push(`  #${d.id} ${d.text}${d.why ? `\n     why: ${d.why}` : ""}${d.action ? `\n     action: ${d.action}` : ""}`);
  }
  const hidden = Number(rows[0]!.total) - rows.length;
  if (hidden > 0) lines.push(`(${hidden} more not shown; raise limit)`);
  return lines.join("\n");
}
