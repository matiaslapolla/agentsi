import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { FailureClass, ReadyAgent } from "./audit.ts";
import { getPool, inTransaction, type Queryable } from "./db.ts";
import { git } from "./exec.ts";
import { dataRoot } from "./paths.ts";
import type { ReplayReport } from "./replay.ts";

export type SignalKind =
  | "first_check_failure"
  | "unresolved_source"
  | "invented_source"
  | "error"
  | "sin_datos"
  | "mark_accepted"
  | "mark_changed"
  | "mark_rejected"
  | "decision_rejected"
  | "reverted_reflection";

export interface Signal {
  agent: string;
  kind: SignalKind;
  /** A minutes id, or a reflection id for `reverted_reflection`. */
  ref: string;
  detail: string;
}

export interface MemoryChange {
  agent: string;
  added: string[];
  retired: string[];
  helpful: string[];
  harmful: string[];
  /** Items a revert brought back from retirement. */
  restored?: string[];
  /** Deltas the code refused, with the reason. */
  dropped: string[];
  diff: string;
}

export interface Proposal {
  agent: string;
  /** Repo-relative: `workspace/<p>/agents/<agent>.md`, or `plugin/agents/<role>.md` for a global change. */
  target: string;
  /** The threshold the agent crossed. */
  trigger: { reason: ReadyAgent["reason"]; failure_class: FailureClass | null; count: number };
  /** Set for a global target: the lesson that holds in two or more products. */
  shared_lesson: { text: string; products: string[] } | null;
  rationale: string;
  expected_effect: string;
  /** Unified diff of the target, for display. */
  diff: string;
  /** The full text the target gets on approval. */
  content: string;
  /** sha256 of the normalized new text; reflect never proposes it again once rejected or discarded. */
  body_hash: string;
  /** sha256 of the target file when the proposal was made; null when it did not exist. */
  target_hash: string | null;
  replay: ReplayReport;
  decision_id: number | null;
  status: "pending" | "approved" | "rejected" | "discarded" | "stale";
}

/** One agent past the proposal threshold, and what reflect did about it. */
export interface ProposalCandidate {
  agent: string;
  reason: ReadyAgent["reason"];
  failure_class: FailureClass | null;
  count: number;
  /** Replay cases, and how many of them are held out. */
  cases: number;
  held_out: number;
  /** `deferred`: another agent got this reflection's proposal; its evidence waits for the next one. */
  outcome: "proposed" | "insufficient_evidence" | "deferred" | "failed";
  note: string | null;
}

export interface ReflectionEntry {
  id: string;
  product: string;
  created_at: string;
  trigger: "manual";
  /** Set on the line a revert appends: the id of the reverted entry. */
  reverts?: string;
  signals: Signal[];
  memory: MemoryChange[];
  proposal: Proposal | null;
  /** Agents past the proposal threshold; absent when none was. */
  candidates?: ProposalCandidate[];
  /**
   * Minutes ids (ledger lines, minutes files) and memory item sources this reflection processed first; later
   * reflections treat everything else as new. Absent on lines written before the field, and on reverts.
   */
  consumed?: string[];
  status: "applied" | "reverted";
  /** Always null: a commit cannot hold its own sha. The commit is the one that added this line. */
  commit: string | null;
  cost_usd?: number;
}

async function selectReflections(db: Queryable, product: string, warn: (msg: string) => void): Promise<ReflectionEntry[]> {
  const { rows } = await db.query<{ entry: unknown }>("select entry from reflections where product = $1 order by position", [product]);
  const out: ReflectionEntry[] = [];
  for (const r of rows) {
    try {
      out.push((typeof r.entry === "string" ? JSON.parse(r.entry) : r.entry) as ReflectionEntry);
    } catch {
      warn("reflections row skipped: not JSON");
    }
  }
  return out;
}

/** The product's reflection entries, oldest first. A stored row that is not JSON is reported through `warn` and skipped. The data root argument is ignored. */
export function readReflections(product: string, _root = dataRoot, warn: (msg: string) => void = console.warn): Promise<ReflectionEntry[]> {
  return selectReflections(getPool(), product, warn);
}

/** Makes the stored entries of the product exactly `entries`, in that order. */
async function saveReflections(db: Queryable, product: string, entries: ReflectionEntry[]): Promise<void> {
  await db.query("delete from reflections where product = $1 and id <> all($2::text[])", [product, entries.map((e) => e.id)]);
  for (const [position, e] of entries.entries()) {
    await db.query(
      `insert into reflections (product, id, position, entry) values ($1, $2, $3, $4::jsonb)
       on conflict (product, id) do update set position = excluded.position, entry = excluded.entry`,
      [product, e.id, position, JSON.stringify(e)],
    );
  }
}

export async function writeReflections(product: string, entries: ReflectionEntry[], _root = dataRoot): Promise<void> {
  await inTransaction(await getPool().connect(), (db) => saveReflections(db, product, entries));
}

/** An applied entry whose memory changed something and that no later line has reverted. */
export function canRevert(entry: Pick<ReflectionEntry, "reverts" | "status" | "memory">): boolean {
  return !entry.reverts && entry.status === "applied" && entry.memory.some((m) => m.diff.trim());
}

/** Re-reads the entries under a per-product transaction lock and writes back what `change` returns, so a concurrent writer's entries survive. */
export async function updateReflections(product: string, change: (entries: ReflectionEntry[]) => ReflectionEntry[], _root = dataRoot): Promise<void> {
  await inTransaction(await getPool().connect(), async (db) => {
    await db.query("select pg_advisory_xact_lock(hashtext($1))", [`reflections:${product}`]);
    await saveReflections(db, product, change(await selectReflections(db, product, console.warn)));
  });
}

/** Unified diff of two texts through `git diff --no-index`; empty when they are equal. */
export async function unifiedDiff(before: string, after: string, label: string): Promise<string> {
  if (before === after) return "";
  const dir = mkdtempSync(join(tmpdir(), "agentsi-diff-"));
  try {
    writeFileSync(join(dir, "a"), before ? before + "\n" : "");
    writeFileSync(join(dir, "b"), after ? after + "\n" : "");
    const res = await git(dir, "diff", "--no-index", "--no-color", "--no-ext-diff", "-U3", "a", "b");
    const start = res.stdout.indexOf("@@");
    return start < 0 ? "" : `--- a/${label}\n+++ b/${label}\n${res.stdout.slice(start)}`;
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}
