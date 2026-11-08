import fs from "node:fs";
import path from "node:path";
import type { Minutes as StoredMinutes } from "@agentsi/core";
import { WORKSPACE } from "./env.ts";
import { rows } from "./db.ts";
import type { DecisionKind } from "./decisions.ts";
import { findProduct } from "./registry.ts";
import type {
  MarkSummary,
  MarkValue,
  Minutes,
  Priority,
  Timing,
  UsageRow,
  UsageTotals,
} from "./types.ts";

const SAFE = /^[A-Za-z0-9_][A-Za-z0-9._-]*$/;

export function safeSegment(s: string): string | null {
  return SAFE.test(s) && !s.includes("..") ? s : null;
}

export function productDir(product: string): string | null {
  const id = safeSegment(product);
  return id && findProduct(id) ? path.join(WORKSPACE, id) : null;
}

function readJson<T>(file: string): T | null {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8")) as T;
  } catch {
    return null;
  }
}

interface DbMark {
  minutes_id: string;
  idx: number;
  mark: MarkValue | null;
  is_new: boolean;
}

async function dbMarks(ids: string[]): Promise<Map<string, DbMark>> {
  if (!ids.length) return new Map();
  const found = await rows<DbMark>(
    "select minutes_id, idx, mark, is_new from priorities where minutes_id = any($1)",
    [ids],
  );
  return new Map(found.map((r) => [`${r.minutes_id}:${r.idx}`, r]));
}

function applyMarks(m: Minutes, marks: Map<string, DbMark>): Minutes {
  const priorities = (m.priorities ?? []).map((p, i): Priority => {
    const db = marks.get(`${m.id}:${i}`);
    return db ? { ...p, mark: db.mark, is_new: db.is_new } : p;
  });
  return { ...m, priorities };
}

export function summarizeMarks(priorities: Priority[] = []): MarkSummary {
  const s: MarkSummary = { accepted: 0, changed: 0, rejected: 0, unmarked: 0, fresh: 0 };
  for (const p of priorities) {
    if (p.mark) s[p.mark] += 1;
    else s.unmarked += 1;
    if (p.is_new) s.fresh += 1;
  }
  return s;
}

const parseData = <T>(v: unknown): T | null => {
  try {
    return (typeof v === "string" ? JSON.parse(v) : v) as T;
  } catch {
    return null;
  }
};

/** The archived refs of one kind, read through the shared pool. An unreachable database archives nothing. */
export async function archivedRefs(product: string, kind: "minutes" | "decision" | "product"): Promise<Set<string>> {
  const found = await rows<{ ref: string }>("select ref from archive_records where product = $1 and kind = $2", [product, kind]);
  return new Set(found.map((r) => String(r.ref)));
}

/** Archived minutes are left out unless `includeArchived` is set; then they carry `archived: true`. */
export async function listMinutes(product: string, opts: { includeArchived?: boolean } = {}): Promise<Minutes[]> {
  if (!productDir(product)) return [];
  const stored = await rows<{ data: unknown }>("select data from minutes where product = $1 and data is not null", [product]);
  const all = stored.map((r) => parseData<Minutes>(r.data)).filter((m): m is Minutes => !!m && typeof m.id === "string");
  const archived = await archivedRefs(product, "minutes");
  const kept = opts.includeArchived ? all : all.filter((m) => !archived.has(m.id));
  const marks = await dbMarks(kept.map((m) => m.id));
  return kept
    .map((m) => ({ ...applyMarks(m, marks), ...(archived.has(m.id) ? { archived: true } : {}) }))
    .sort((a, b) => b.id.localeCompare(a.id));
}

/** The minutes JSON of the database, with the marks of the priorities table. The Markdown is rendered from it by the reader. */
export async function getMinutes(product: string, id: string): Promise<(StoredMinutes & { archived?: boolean }) | null> {
  const safe = safeSegment(id);
  if (!productDir(product) || !safe) return null;
  const [row] = await rows<{ data: unknown }>("select data from minutes where product = $1 and id = $2 and data is not null", [product, safe]);
  const m = row ? parseData<StoredMinutes>(row.data) : null;
  if (!m) return null;
  // The row holds the full minutes JSON, so the web type only narrows what a reader may rely on.
  const marked = applyMarks(m as Minutes, await dbMarks([m.id])) as StoredMinutes;
  return (await archivedRefs(product, "minutes")).has(m.id) ? { ...marked, archived: true } : marked;
}

/** The per-role usage stored with the minutes; empty when there is none. */
export async function getUsage(product: string, id: string): Promise<UsageRow[]> {
  const safe = safeSegment(id);
  if (!productDir(product) || !safe) return [];
  const [row] = await rows<{ usage: unknown }>("select usage from minutes where product = $1 and id = $2", [product, safe]);
  const u = row ? parseData<UsageRow[]>(row.usage) : null;
  return Array.isArray(u) ? u : [];
}

export function sumUsage(list: UsageRow[]): UsageTotals {
  const t: UsageTotals = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, cost: 0, rows: list.length };
  for (const u of list) {
    t.input += Number(u.input_tokens) || 0;
    t.output += Number(u.output_tokens) || 0;
    t.cacheRead += Number(u.cache_read) || 0;
    t.cacheWrite += Number(u.cache_write) || 0;
    t.cost += Number(u.cost_usd) || 0;
  }
  return t;
}

export async function productUsage(product: string): Promise<UsageTotals> {
  const [db] = await rows<{
    n: number;
    i: string;
    o: string;
    cr: string;
    cw: string;
    c: string;
  }>(
    `select count(*)::int n, coalesce(sum(u.input_tokens),0) i, coalesce(sum(u.output_tokens),0) o,
            coalesce(sum(u.cache_read),0) cr, coalesce(sum(u.cache_write),0) cw, coalesce(sum(u.cost_usd),0) c
       from usage u join minutes m on m.id = u.minutes_id where m.product = $1`,
    [product],
  );
  if (db && db.n > 0) {
    return { rows: db.n, input: Number(db.i), output: Number(db.o), cacheRead: Number(db.cr), cacheWrite: Number(db.cw), cost: Number(db.c) };
  }
  return sumUsage([]);
}

export interface PendingDecision {
  id: number;
  kind: DecisionKind;
  /** Null for reflection and product decisions. */
  minutes_id: string | null;
  reflection_id: string | null;
  text: string;
  why: string | null;
  action: string | null;
  archived?: boolean;
}

/** Archived decisions are left out unless `includeArchived` is set; then they carry `archived: true`. */
export async function pendingDecisions(product: string, opts: { includeArchived?: boolean } = {}): Promise<PendingDecision[]> {
  const all = await rows<PendingDecision>(
    "select id, kind, minutes_id, reflection_id, text, why, action from decisions where product = $1 and status = 'pending' order by id",
    [product],
  );
  const archived = await archivedRefs(product, "decision");
  if (opts.includeArchived) return all.map((d) => (archived.has(String(d.id)) ? { ...d, archived: true } : d));
  return all.filter((d) => !archived.has(String(d.id)));
}

export interface DecisionRow extends PendingDecision {
  status: "pending" | "approved" | "rejected";
}

export function decisionsFor(minutesId: string): Promise<DecisionRow[]> {
  return rows<DecisionRow>(
    "select id, kind, minutes_id, reflection_id, text, why, action, status from decisions where minutes_id = $1 order by id",
    [minutesId],
  );
}

export function audioFile(product: string, id: string): string | null {
  const dir = productDir(product);
  const safe = safeSegment(id);
  if (!dir || !safe) return null;
  const f = path.join(dir, "audio", safe, "call.webm");
  return fs.existsSync(f) ? f : null;
}

export function getTimings(product: string, id: string): Timing[] {
  const dir = productDir(product);
  const safe = safeSegment(id);
  if (!dir || !safe) return [];
  const t = readJson<Timing[]>(path.join(dir, "audio", safe, "timings.json"));
  return Array.isArray(t) ? t : [];
}

export interface MemoryItem {
  id: string;
  text: string;
  kind: "fact" | "lesson";
  helpful: number;
  harmful: number;
  source: string;
  created_at: string;
  retired_at: string | null;
  retired_why: string | null;
}

export interface MemoryEntry {
  role: string;
  items: MemoryItem[] | null;
  history: { hash: string; date: string; subject: string }[];
}

/** Memory lives in the `memory` table, so it has no git history; `history` stays empty. */
export async function memoryFor(product: string, roles: string[]): Promise<MemoryEntry[]> {
  if (!productDir(product)) return [];
  return Promise.all(
    roles.map(async (role) => {
      const found = await rows<MemoryItem>(
        "select id, text, kind, helpful, harmful, source, created_at, retired_at, retired_why from memory where product = $1 and agent = $2 order by position",
        [product, role],
      );
      const items = found.map((i) => ({ ...i, helpful: Number(i.helpful), harmful: Number(i.harmful) }));
      return { role, items: items.length ? items : null, history: [] };
    }),
  );
}
