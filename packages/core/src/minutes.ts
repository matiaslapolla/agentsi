import { existsSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { getPool, inTransaction, type Queryable, type TxClient } from "./db.ts";
import type { CommitResult } from "./exec.ts";
import { indexWorkspace } from "./indexer.ts";
import { writeLedger, type MeetingLogEntry } from "./meeting-log.ts";
import { findMinutes, saveMinutesData } from "./minutes-store.ts";
import { appendFacts } from "./memory.ts";
import { dataRoot, workspaceRoot } from "./paths.ts";
import { MissingProposal, settleReflection, type DecisionStore } from "./reflection-decision.ts";
import type { DecisionStatus, Mark, Minutes, UsageRow } from "./types.ts";

export { markBox, renderMinutesMarkdown } from "./minutes-md.ts";
export { readMinutes, findMinutes, readUsage } from "./minutes-store.ts";
export type { CommitResult } from "./exec.ts";

/** What a change reports in place of a commit: the data lives in Postgres and no flow commits in the data root. */
const NO_COMMIT: CommitResult = { committed: false };

export function newMinutesId(question: string, date = new Date()): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  const stamp = `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}-${pad(date.getHours())}${pad(date.getMinutes())}`;
  const slug = question
    .normalize("NFD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40)
    .replace(/-+$/, "");
  return `${stamp}-${slug || "meeting"}`;
}

/**
 * The minutes file of `id` that an old data root still holds, in any product's workspace. New minutes are never
 * written to a file; this only keeps a new id from colliding with an old file that was not imported yet.
 */
export function minutesFileOf(id: string, root = dataRoot): { file: string; product: string } | null {
  for (const product of existsSync(workspaceRoot(root)) ? readdirSync(workspaceRoot(root)) : []) {
    const file = join(workspaceRoot(root), product, "minutes", `${id}.json`);
    if (existsSync(file)) return { file, product };
  }
  return null;
}

/**
 * Claims `m.id`, or `m.id-2`, `m.id-3`..., for `m.product`: the first candidate with no old minutes file whose row
 * is free or already this product's. The row commits before anything else is written, so two meetings that start in
 * the same minute with the same question cannot both claim one id.
 */
export async function reserveMinutesId(
  m: Pick<Minutes, "id" | "product" | "scope" | "question" | "status" | "created_at">,
  o: { root?: string; db?: DecisionStore } = {},
): Promise<string> {
  const db = o.db ?? getPool();
  for (let n = 1; ; n++) {
    const id = n === 1 ? m.id : `${m.id}-${n}`;
    if (minutesFileOf(id, o.root)) continue;
    const { rows } = await db.query<{ product: string }>(
      `insert into minutes (id, product, scope, question, status, created_at) values ($1,$2,$3,$4,$5,$6)
       on conflict (id) do update set id = minutes.id returning product`,
      [id, m.product, m.scope, m.question, m.status, m.created_at],
    );
    if (rows[0]?.product === m.product) return id;
  }
}

/** Writes the minutes row, its priorities, decisions and usage rows, and the meeting log line and snapshot when given, in one transaction. */
export async function storeMinutes(m: Minutes, usage: UsageRow[] | null, ledger?: { entry: MeetingLogEntry; snapshot: string }): Promise<void> {
  await inTransaction(await getPool().connect(), async (client) => {
    const holder = await client.query<{ product: string }>("select product from minutes where id = $1 for update", [m.id]);
    if (holder.rows[0] && holder.rows[0].product !== m.product) {
      throw new Error(`Minutes id ${m.id} belongs to ${holder.rows[0].product}, not ${m.product}`);
    }
    await client.query("delete from priorities where minutes_id = $1", [m.id]);
    await client.query("delete from decisions where minutes_id = $1", [m.id]);
    if (usage) await client.query("delete from usage where minutes_id = $1", [m.id]);
    await client.query(
      `insert into minutes (id, product, scope, question, status, created_at, data, usage) values ($1,$2,$3,$4,$5,$6,$7::jsonb,$8::jsonb)
       on conflict (id) do update set product = excluded.product, scope = excluded.scope, question = excluded.question,
         status = excluded.status, created_at = excluded.created_at, data = excluded.data, usage = coalesce(excluded.usage, minutes.usage)`,
      [m.id, m.product, m.scope, m.question, m.status, m.created_at, JSON.stringify(m), usage ? JSON.stringify(usage) : null],
    );
    for (const [i, p] of m.priorities.entries()) {
      await client.query(
        "insert into priorities (minutes_id, idx, title, why, sources, owner_role, objective, mark, is_new) values ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        [m.id, i, p.title, p.why, JSON.stringify(p.sources), p.owner_role, p.objective ?? null, p.mark, p.is_new],
      );
    }
    for (const d of m.decisions) {
      await client.query(
        "insert into decisions (product, minutes_id, text, why, action, status, decided_at) values ($1,$2,$3,$4,$5,$6,$7)",
        [m.product, m.id, d.text, d.why, d.action ?? null, d.status ?? "pending", d.decided_at ?? null],
      );
    }
    for (const u of usage ?? []) {
      await client.query(
        "insert into usage (minutes_id, role, model, input_tokens, output_tokens, cache_read, cache_write, cost_usd, duration_ms) values ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        [m.id, u.role, u.model, u.input_tokens, u.output_tokens, u.cache_read, u.cache_write, u.cost_usd, Math.round(u.duration_ms)],
      );
    }
    if (ledger) await writeLedger(m.product, ledger.entry, ledger.snapshot, client);
  });
}

/** Stores the minutes JSON, the per-role usage, the meeting log line and the snapshot in Postgres. The Markdown is rendered on read; no file is written. */
export async function writeMinutes(m: Minutes, usage: UsageRow[], ledger: { entry: MeetingLogEntry; snapshot: string }): Promise<void> {
  for (const [role, r] of Object.entries(m.roles)) await appendFacts(m.product, role, r.memory_notes, m.id, new Date(m.created_at));
  await storeMinutes(m, usage, ledger);
  await indexWorkspace(m.product);
}

async function loadMinutes(minutesId: string): Promise<Minutes> {
  const found = await findMinutes(minutesId);
  if (found) return found;
  throw new Error(`Minutes "${minutesId}" not found`);
}

/** `unmarkedOnly` refuses a priority that another mark reached first. The same mark again passes, so a retry works. */
export interface MarkOptions {
  unmarkedOnly?: boolean;
}

export interface MarkEntry {
  minutesId: string;
  idx: number;
  mark: Mark;
  isNew: boolean;
}

/**
 * Sets every mark in memory. Throws before any change when one entry points at a missing minutes or priority,
 * repeats a priority, or, with `unmarkedOnly`, hits a priority that already has another mark.
 */
export function setMarks(byId: Map<string, Minutes>, entries: MarkEntry[], o: MarkOptions = {}): void {
  const seen = new Set<string>();
  for (const e of entries) {
    const m = byId.get(e.minutesId);
    if (!m) throw new Error(`Minutes "${e.minutesId}" not found`);
    const p = m.priorities[e.idx];
    if (!p) throw new Error(`Minutes ${e.minutesId} has no priority #${e.idx} (0..${m.priorities.length - 1})`);
    const key = `${e.minutesId}#${e.idx}`;
    if (seen.has(key)) throw new Error(`Priority ${key} appears twice in one batch`);
    seen.add(key);
    if (o.unmarkedOnly && p.mark && (p.mark !== e.mark || p.is_new !== e.isNew)) throw new Error(`Priority ${key} is already marked ${p.mark}`);
  }
  for (const e of entries) {
    const p = byId.get(e.minutesId)!.priorities[e.idx]!;
    p.mark = e.mark;
    p.is_new = e.isNew;
  }
}

export interface MarkBatch {
  entries: MarkEntry[];
  before: Map<string, Minutes>;
  after: Map<string, Minutes>;
}

/**
 * Updates the priority rows and the minutes JSON in one transaction. `write` replaces the default write of the JSON; when it
 * writes outside the database, a failure rolls the rows back and gives the written minutes their old content.
 */
export async function persistMarks(
  batch: MarkBatch,
  client: TxClient,
  o: MarkOptions = {},
  write: (m: Minutes, db: Queryable) => unknown = saveMinutesData,
): Promise<void> {
  const touched: Minutes[] = [];
  const outside = write !== saveMinutesData;
  try {
    await inTransaction(client, async (db) => {
      for (const e of batch.entries) {
        const params = [e.minutesId, e.idx, e.mark, e.isNew];
        const guard = o.unmarkedOnly ? " and (mark is null or (mark = $3 and is_new = $4))" : "";
        const { rows } = await db.query(`update priorities set mark = $3, is_new = $4 where minutes_id = $1 and idx = $2${guard} returning idx`, params);
        if (o.unmarkedOnly && !rows.length) {
          const row = await db.query("select 1 from priorities where minutes_id = $1 and idx = $2", params.slice(0, 2));
          if (row.rows.length) throw new Error(`Priority ${e.minutesId}#${e.idx} is already marked`);
        }
      }
      for (const m of batch.after.values()) {
        touched.push(m);
        await write(m, db);
      }
    });
  } catch (err) {
    for (const m of outside ? touched : []) {
      try {
        await write(batch.before.get(m.id)!, client);
      } catch {
        // The original error matters more than a failed restore.
      }
    }
    throw err;
  }
}

/** Keeps the first error, else reports a commit when any product committed. */
export function combineCommits(results: CommitResult[]): CommitResult {
  return results.find((r) => r.error) ?? { committed: results.some((r) => r.committed) };
}

/**
 * Marks several priorities in one transaction. A throw means nothing was written. A failure after the write
 * (the re-index) comes back as the error of the result. Nothing is committed: the data lives in Postgres.
 */
export async function applyMarks(entries: MarkEntry[], o: MarkOptions = {}): Promise<CommitResult> {
  if (!entries.length) return { committed: false };
  const byId = new Map<string, Minutes>();
  for (const id of new Set(entries.map((e) => e.minutesId))) byId.set(id, await loadMinutes(id));
  const before = structuredClone(byId);
  setMarks(byId, entries, o);
  await persistMarks({ entries, before, after: byId }, await getPool().connect(), o);
  const results: CommitResult[] = [];
  for (const product of new Set([...byId.values()].map((m) => m.product))) {
    try {
      await indexWorkspace(product);
      results.push(NO_COMMIT);
    } catch (err) {
      results.push({ committed: false, error: (err as Error).message });
    }
  }
  return combineCommits(results);
}

export function applyMark(minutesId: string, idx: number, mark: Mark, isNew = false): Promise<CommitResult> {
  return applyMarks([{ minutesId, idx, mark, isNew }]);
}

/** A pending decision outside any meeting (`kind = 'product'`), such as setup's north-star proposal. */
export async function addProductDecision(product: string, text: string, why: string, action?: string, db: DecisionStore = getPool()): Promise<number> {
  const { rows } = await db.query<{ id: string }>(
    "insert into decisions (product, kind, minutes_id, text, why, action, status) values ($1, 'product', null, $2, $3, $4, 'pending') returning id",
    [product, text, why, action || null],
  );
  return Number(rows[0]!.id);
}

/** The line `agentsi decide` prints so the agent can run what the owner approved. */
export function actionLine(res: { status: string; action: string | null }): string | null {
  return res.status === "approved" && res.action ? `Action: ${res.action}` : null;
}

export async function decide(
  decisionId: number,
  status: Exclude<DecisionStatus, "pending">,
  o: { db?: DecisionStore; root?: string } = {},
): Promise<{ product: string; minutesId: string | null; status: Exclude<DecisionStatus, "pending"> | "stale"; action: string | null; commit: CommitResult }> {
  const pool = o.db ?? getPool();
  const { rows: found } = await pool.query<{ product: string; kind: string; reflection_id: string | null }>(
    "select product, kind, reflection_id from decisions where id = $1",
    [decisionId],
  );
  const target = found[0];
  if (!target) throw new Error(`Decision ${decisionId} not found`);
  if (target.kind === "reflection") {
    if (!target.reflection_id) throw new Error(`Decision ${decisionId} is a reflection decision without a reflection id`);
    const settled = await settleReflection(target.product, target.reflection_id, status, { root: o.root }).catch(async (err: unknown) => {
      if (!(err instanceof MissingProposal)) throw err;
      await pool.query("update decisions set status = 'rejected', decided_at = now() where id = $1", [decisionId]);
      throw new Error(`Decision ${decisionId} rejected: ${err.message} It can never be applied.`);
    });
    await pool.query("update decisions set status = $2, decided_at = now() where id = $1", [decisionId, settled.status]);
    return { product: target.product, minutesId: null, status: settled.status, action: null, commit: settled.commit };
  }

  const { rows } = await pool.query<{ product: string; minutes_id: string | null; action: string | null; pos: string }>(
    `update decisions set status = $2, decided_at = now() where id = $1
     returning product, minutes_id, action, (select count(*) from decisions d2 where d2.minutes_id = decisions.minutes_id and d2.id < decisions.id) as pos`,
    [decisionId, status],
  );
  const row = rows[0];
  if (!row) throw new Error(`Decision ${decisionId} not found`);
  if (!row.minutes_id) return { product: row.product, minutesId: null, status, action: row.action, commit: { committed: false } };
  const m = await loadMinutes(row.minutes_id);
  const d = m.decisions[Number(row.pos)];
  if (d) {
    d.status = status;
    d.decided_at = new Date().toISOString();
    await saveMinutesData(m);
    await indexWorkspace(m.product);
  }
  return { product: row.product, minutesId: row.minutes_id, status, action: row.action, commit: NO_COMMIT };
}
