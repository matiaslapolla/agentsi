import { randomBytes } from "node:crypto";
import { getPool, inTransaction, type Queryable } from "./db.ts";
import { AGENT_ID } from "./roles.ts";

export const MEMORY_MAX_CHARS = 6000;

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

function assertAgent(agent: string): void {
  if (!AGENT_ID.test(agent)) throw new Error(`Invalid agent id "${agent}"`);
}

export const normalizeText = (s: string): string => s.trim().replace(/\s+/g, " ");

export const activeItems = (items: MemoryItem[]): MemoryItem[] => items.filter((i) => i.retired_at === null);

interface MemoryRow {
  id: string;
  text: string;
  kind: MemoryItem["kind"];
  helpful: number | string;
  harmful: number | string;
  source: string;
  created_at: string;
  retired_at: string | null;
  retired_why: string | null;
}

const toItem = (r: MemoryRow): MemoryItem => ({
  id: r.id,
  text: r.text,
  kind: r.kind,
  helpful: Number(r.helpful),
  harmful: Number(r.harmful),
  source: r.source,
  created_at: r.created_at,
  retired_at: r.retired_at,
  retired_why: r.retired_why,
});

async function readItems(db: Queryable, product: string, agent: string): Promise<MemoryItem[]> {
  assertAgent(agent);
  const { rows } = await db.query<MemoryRow & Record<string, unknown>>(
    "select id, text, kind, helpful, harmful, source, created_at, retired_at, retired_why from memory where product = $1 and agent = $2 order by position",
    [product, agent],
  );
  return rows.map(toItem);
}

async function replaceItems(db: Queryable, product: string, agent: string, items: MemoryItem[]): Promise<void> {
  assertAgent(agent);
  await db.query("delete from memory where product = $1 and agent = $2", [product, agent]);
  for (const [position, i] of items.entries()) {
    await db.query(
      `insert into memory (product, agent, id, position, text, kind, helpful, harmful, source, created_at, retired_at, retired_why)
       values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)`,
      [product, agent, i.id, position, i.text, i.kind, i.helpful, i.harmful, i.source, i.created_at, i.retired_at, i.retired_why],
    );
  }
}

/** The items of one agent in the order they were added; empty when it has no memory. */
export function readMemoryItems(product: string, agent: string): Promise<MemoryItem[]> {
  return readItems(getPool(), product, agent);
}

/** Replaces the whole memory of one agent. */
export async function writeMemoryItems(product: string, agent: string, items: MemoryItem[]): Promise<void> {
  assertAgent(agent);
  await inTransaction(await getPool().connect(), (db) => replaceItems(db, product, agent, items));
}

/** Removes the memory of one agent; true when it had any. */
export async function deleteMemory(product: string, agent: string, db: Queryable = getPool()): Promise<boolean> {
  assertAgent(agent);
  const { rows } = await db.query("delete from memory where product = $1 and agent = $2 returning id", [product, agent]);
  return rows.length > 0;
}

/** The agents of a product that hold memory, sorted. */
export async function memoryAgents(product: string): Promise<string[]> {
  const { rows } = await getPool().query<{ agent: string }>("select distinct agent from memory where product = $1 order by agent", [product]);
  return rows.map((r) => r.agent);
}

/** Every (product, agent) that holds memory, sorted. */
export async function allMemoryAgents(): Promise<{ product: string; agent: string }[]> {
  const { rows } = await getPool().query<{ product: string; agent: string }>("select distinct product, agent from memory order by product, agent");
  return rows;
}

function newItemId(taken: Iterable<string>): string {
  const used = new Set(taken);
  for (const bytes of [2, 2, 2, 2, 2, 3]) {
    const id = `m-${randomBytes(bytes).toString("hex")}`;
    if (!used.has(id)) return id;
  }
  throw new Error("Could not allocate a memory item id");
}

export function makeItem(
  text: string,
  o: { kind?: MemoryItem["kind"]; source: string; created_at: string; taken: Iterable<string> },
): MemoryItem {
  return {
    id: newItemId(o.taken),
    text: normalizeText(text),
    kind: o.kind ?? "fact",
    helpful: 0,
    harmful: 0,
    source: o.source,
    created_at: o.created_at,
    retired_at: null,
    retired_why: null,
  };
}

/** Adds each note as a fact item, skipping texts equal (after whitespace normalization) to an active item. */
export function addFacts(items: MemoryItem[], notes: string[], source: string, now: Date): MemoryItem[] {
  const out = [...items];
  const seen = new Set(activeItems(out).map((i) => normalizeText(i.text)));
  for (const note of notes) {
    const text = normalizeText(note);
    if (!text || seen.has(text)) continue;
    seen.add(text);
    out.push(makeItem(text, { source, created_at: now.toISOString(), taken: out.map((i) => i.id) }));
  }
  return out;
}

export async function appendFacts(product: string, agent: string, notes: string[], source: string, now = new Date()): Promise<number> {
  assertAgent(agent);
  return inTransaction(await getPool().connect(), async (db) => {
    await db.query("select pg_advisory_xact_lock(hashtext($1))", [`memory:${product}:${agent}`]);
    const items = await readItems(db, product, agent);
    const next = addFacts(items, notes, source, now);
    if (next.length === items.length) return 0;
    await replaceItems(db, product, agent, next);
    return next.length - items.length;
  });
}

function mapItem(items: MemoryItem[], id: string, change: (i: MemoryItem) => MemoryItem): MemoryItem[] {
  if (!items.some((i) => i.id === id)) throw new Error(`No memory item "${id}"`);
  return items.map((i) => (i.id === id ? change(i) : i));
}

export function retireItem(items: MemoryItem[], id: string, why: string, now: Date): MemoryItem[] {
  return mapItem(items, id, (i) => ({ ...i, retired_at: now.toISOString(), retired_why: why }));
}

export function bumpCounter(items: MemoryItem[], id: string, counter: "helpful" | "harmful"): MemoryItem[] {
  return mapItem(items, id, (i) => ({ ...i, [counter]: i[counter] + 1 }));
}

const newestFirst = (items: MemoryItem[]): MemoryItem[] =>
  items
    .map((item, idx) => ({ item, idx }))
    .sort((a, b) => (a.item.created_at < b.item.created_at ? 1 : a.item.created_at > b.item.created_at ? -1 : b.idx - a.idx))
    .map((x) => x.item);

/** Active items, newest first, as `- [id] text`. Items that do not fit are counted in a last line; none is cut. */
export function renderForPrompt(items: MemoryItem[], maxChars: number): string {
  const lines = newestFirst(activeItems(items)).map((i) => `- [${i.id}] ${i.text}`);
  const withCount = (kept: string[]): string => {
    const hidden = lines.length - kept.length;
    return [...kept, ...(hidden ? [`(${hidden} older ${hidden === 1 ? "item" : "items"} not shown)`] : [])].join("\n");
  };
  let kept = lines.length;
  while (kept > 0 && withCount(lines.slice(0, kept)).length > maxChars) kept--;
  return withCount(lines.slice(0, kept));
}

export async function memoryForPrompt(product: string, agent: string): Promise<string> {
  return renderForPrompt(await readMemoryItems(product, agent), MEMORY_MAX_CHARS);
}
