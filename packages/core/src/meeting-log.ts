import { createHash } from "node:crypto";
import { appendFileSync, mkdirSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";
import { getPool, type Queryable } from "./db.ts";
import { dataRoot } from "./paths.ts";
import type { Minutes, Source, UsageRow } from "./types.ts";

export interface CheckOutcome {
  passed: boolean;
  failures: string[];
}

export interface AgentLogEntry {
  prompt_hash: string;
  memory_hash: string;
  model: string;
  cost_usd: number;
  duration_ms: number;
  error: string | null;
  priorities: number;
  sources: number;
  unresolved_sources: Source[];
  sin_datos: number;
}

export interface MeetingLogEntry {
  minutes_id: string;
  created_at: string;
  question: string;
  scope: string | null;
  brief_hash: string | null;
  agents: Record<string, AgentLogEntry>;
  synthesis: {
    first_check: CheckOutcome;
    repaired: boolean;
    final_check: CheckOutcome;
    invented_sources: Source[];
    cost_usd: number;
  };
  total_cost_usd: number;
}

/** What `runMeeting` knows about one agent that the minutes do not hold. */
export interface AgentAnswerLog {
  prompt: string;
  memory: string;
  model: string;
  error: string | null;
  duration_ms: number;
  unresolved_sources: Source[];
}

export interface LogEntryInput {
  minutes: Minutes;
  usage: UsageRow[];
  agents: Record<string, AgentAnswerLog>;
  snapshot: string;
  briefHash?: string | null;
}

export const sha256 = (text: string): string => createHash("sha256").update(text).digest("hex");

/** The marker the role prompt asks for when data is missing, in any answer language. */
export const NO_DATA = "[no data]";

/** Also counts "sin datos", which the role prompt asked for before the marker. */
export const sinDatos = (text: string): number => text.match(/\[no data\]|sin datos/gi)?.length ?? 0;

const sameSource = (a: Source, b: Source): boolean => a.type === b.type && a.ref === b.ref;

const cost = (rows: UsageRow[]): number => rows.reduce((n, u) => n + u.cost_usd, 0);

/** Sources in the synthesis that no agent gave and that are not in the snapshot. */
export function inventedSources(m: Pick<Minutes, "roles" | "priorities">, snapshot: string): Source[] {
  const given = Object.values(m.roles).flatMap((r) => r.priorities.flatMap((p) => p.sources));
  const invented: Source[] = [];
  for (const s of m.priorities.flatMap((p) => p.sources)) {
    if (given.some((g) => sameSource(g, s)) || snapshot.includes(s.ref) || invented.some((x) => sameSource(x, s))) continue;
    invented.push(s);
  }
  return invented;
}

export function buildLogEntry(i: LogEntryInput): MeetingLogEntry {
  const { minutes: m } = i;
  const agents: Record<string, AgentLogEntry> = {};
  for (const [name, run] of Object.entries(i.agents)) {
    const answer = m.roles[name];
    const priorities = answer?.priorities ?? [];
    agents[name] = {
      prompt_hash: sha256(run.prompt),
      memory_hash: sha256(run.memory),
      model: run.model,
      cost_usd: cost(i.usage.filter((u) => u.role === name)),
      duration_ms: run.duration_ms,
      error: run.error,
      priorities: priorities.length,
      sources: priorities.reduce((n, p) => n + p.sources.length, 0),
      unresolved_sources: run.unresolved_sources,
      sin_datos: sinDatos([answer?.answer ?? "", ...(answer?.concerns ?? [])].join("\n")),
    };
  }

  const firstFailures = m.check.first_failures ?? m.check.failures;

  return {
    minutes_id: m.id,
    created_at: m.created_at,
    question: m.question,
    scope: m.scope,
    brief_hash: i.briefHash ?? null,
    agents,
    synthesis: {
      first_check: { passed: firstFailures.length === 0, failures: firstFailures },
      repaired: m.check.repaired,
      final_check: { passed: m.check.passed, failures: m.check.failures },
      invented_sources: inventedSources(m, i.snapshot),
      cost_usd: cost(i.usage.filter((u) => u.role.startsWith("synthesis"))),
    },
    total_cost_usd: cost(i.usage),
  };
}

/** Appends a meeting to a ledger file and writes its snapshot file. Only file tools use it; the app stores both in Postgres. */
export function appendLedger(logFile: string, snapshotFile: string, entry: MeetingLogEntry, snapshot: string): void {
  mkdirSync(dirname(logFile), { recursive: true });
  mkdirSync(dirname(snapshotFile), { recursive: true });
  writeFileSync(snapshotFile, snapshot);
  appendFileSync(logFile, JSON.stringify(entry) + "\n");
}

/** Stores the meeting log line and the frozen snapshot of one meeting. A second write of the same meeting replaces both. */
export async function writeLedger(product: string, entry: MeetingLogEntry, snapshot: string, db: Queryable = getPool()): Promise<void> {
  await db.query(
    `insert into meeting_log (product, minutes_id, entry) values ($1, $2, $3::jsonb)
     on conflict (product, minutes_id) do update set entry = excluded.entry`,
    [product, entry.minutes_id, JSON.stringify(entry)],
  );
  await db.query(
    `insert into snapshots (product, minutes_id, body) values ($1, $2, $3)
     on conflict (product, minutes_id) do update set body = excluded.body`,
    [product, entry.minutes_id, snapshot],
  );
}

/** The frozen snapshot of a meeting, or null when none was stored. */
export async function readSnapshot(product: string, minutesId: string, db: Queryable = getPool()): Promise<string | null> {
  const { rows } = await db.query<{ body: string }>("select body from snapshots where product = $1 and minutes_id = $2", [product, minutesId]);
  return rows[0]?.body ?? null;
}

export function parseMeetingLog(text: string, warn: (msg: string) => void = console.warn): MeetingLogEntry[] {
  const entries: MeetingLogEntry[] = [];
  for (const [n, line] of text.split("\n").entries()) {
    if (!line.trim()) continue;
    try {
      const e = JSON.parse(line) as MeetingLogEntry;
      if (typeof e.minutes_id !== "string" || typeof e.agents !== "object" || e.agents === null) throw new Error("not a meeting log entry");
      entries.push(e);
    } catch (err) {
      warn(`meetings.jsonl line ${n + 1} skipped: ${(err as Error).message}`);
    }
  }
  return entries;
}

/** The product's meeting log, oldest meeting first. A stored row that is not a log entry is reported through `warn` and skipped. */
export async function readMeetingLog(product: string, _root = dataRoot, warn: (msg: string) => void = console.warn): Promise<MeetingLogEntry[]> {
  const { rows } = await getPool().query<{ entry: unknown }>("select entry from meeting_log where product = $1 order by position", [product]);
  const out: MeetingLogEntry[] = [];
  for (const r of rows) {
    const e = (typeof r.entry === "string" ? JSON.parse(r.entry) : r.entry) as MeetingLogEntry;
    if (typeof e?.minutes_id !== "string" || typeof e.agents !== "object" || e.agents === null) {
      warn("meeting log row skipped: not a meeting log entry");
      continue;
    }
    out.push(e);
  }
  return out;
}
