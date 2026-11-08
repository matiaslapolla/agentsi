export type DecisionKind = "minutes" | "reflection" | "product";

/** Where a pending decision came from: its minutes, the Roles page for a reflection, nothing for a product decision. */
export function decisionLink(
  pid: string,
  d: { kind: DecisionKind; minutes_id: string | null; reflection_id: string | null },
): { href: string; label: string } | null {
  if (d.kind === "reflection") return { href: `/p/${pid}/roles`, label: d.reflection_id ?? "reflexión" };
  if (d.kind === "minutes" && d.minutes_id) return { href: `/p/${pid}/m/${d.minutes_id}`, label: d.minutes_id };
  return null;
}

type Status = "pending" | "approved" | "rejected";

/** Decisions of a minutes file, each with the DB row that has the same text. The file lists the decisions; the DB only adds their status. */
export function withRows<D extends { text: string }, R extends { text: string }>(file: D[], rows: R[]): (D & { row?: R })[] {
  return file.map((d) => ({ ...d, row: rows.find((r) => r.text === d.text) }));
}

/** The DB status wins, then the status written in the file; a decision with neither is pending. */
export function decisionStatus(d: { status?: Status; row?: { status: Status } }): Status {
  return d.row?.status ?? d.status ?? "pending";
}
