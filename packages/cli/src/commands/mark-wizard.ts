import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";
import { actionLine, type CommitResult, type DecisionStatus, type Mark, type MarkEntry, type PendingItem } from "@agentsi/core";

type DecideResult = { status: DecisionStatus | "stale"; minutesId: string | null; action: string | null; commit: CommitResult };

export interface WizardDeps {
  applyMarks(entries: MarkEntry[]): Promise<CommitResult>;
  decide(id: number, status: Exclude<DecisionStatus, "pending">): Promise<DecideResult>;
}

export interface WizardIo {
  input: Readable;
  output: Writable;
  error: Writable;
}

const MARKS: Record<string, Mark> = { a: "accepted", c: "changed", r: "rejected" };

export function describe(item: PendingItem): string[] {
  if (item.kind === "priority") {
    return [
      `Priority ${item.minutes_id} #${item.idx} · owner ${item.owner_role}${item.objective ? ` · objective ${item.objective}` : ""}`,
      `  ${item.title}`,
      `  why: ${item.why}`,
      ...(item.sources.length ? [`  sources: ${item.sources.map((s) => s.ref).join(", ")}`] : []),
    ];
  }
  return [
    `Decision ${item.id}${item.minutes_id ? ` · ${item.minutes_id}` : " · product"}`,
    `  ${item.text}`,
    ...(item.why ? [`  why: ${item.why}`] : []),
    ...(item.action ? [`  action: ${item.action}`] : []),
  ];
}

/** Asks about each pending item, then applies the answers: one commit for the marks, one for each decision. Returns the exit code. */
export async function runWizard(items: PendingItem[], io: WizardIo, deps: WizardDeps): Promise<number> {
  const out = (line = "") => io.output.write(line + "\n");
  if (!items.length) {
    out("Nothing pending.");
    return 0;
  }
  const lines = createInterface({ input: io.input })[Symbol.asyncIterator]();
  /** Resolves to the first accepted key, or null when the input ends. */
  const ask = async (prompt: string, keys: string[]): Promise<string | null> => {
    for (;;) {
      io.output.write(prompt);
      const next = await lines.next();
      if (next.done) return null;
      const key = next.value.trim().toLowerCase();
      if (keys.includes(key)) return key;
    }
  };

  const priorities = items.filter((i) => i.kind === "priority").length;
  out(`${priorities} priorities and ${items.length - priorities} decisions pending.`);
  const marks: MarkEntry[] = [];
  const verdicts: { id: number; status: "approved" | "rejected"; links: MarkEntry[] }[] = [];
  let skipped = 0;
  let stopped = false;
  for (const [n, item] of items.entries()) {
    if (stopped) break;
    out();
    out(`[${n + 1}/${items.length}] ${describe(item)[0]}`);
    for (const line of describe(item).slice(1)) out(line);
    if (item.kind === "priority") {
      const key = await ask("[a]ccept [c]hange [r]eject [s]kip [q]uit > ", ["a", "c", "r", "s", "q"]);
      if (key === null || key === "q") stopped = true;
      else if (key === "s") skipped++;
      else {
        const answer = await ask("New (you did not have it in mind)? [y/N] > ", ["y", "n", ""]);
        if (answer === null) stopped = true;
        else marks.push({ minutesId: item.minutes_id, idx: item.idx, mark: MARKS[key]!, isNew: answer === "y" });
      }
    } else {
      const key = await ask("[a]pprove [r]eject [s]kip [q]uit > ", ["a", "r", "s", "q"]);
      if (key === null || key === "q") stopped = true;
      else if (key === "s") skipped++;
      else {
        const links: MarkEntry[] = key === "a" && item.minutes_id
          ? (item.priorities ?? []).map((idx) => ({ minutesId: item.minutes_id!, idx, mark: "accepted" as const, isNew: false }))
          : [];
        verdicts.push({ id: item.id, status: key === "a" ? "approved" : "rejected", links });
      }
    }
  }

  let code = 0;
  const err = (line: string) => {
    io.error.write(line + "\n");
    code = 1;
  };
  out();
  if (marks.length) {
    try {
      const commit = await deps.applyMarks(marks);
      if (commit.error) io.error.write(`Commit skipped: ${commit.error}\n`);
    } catch (e) {
      err(`Marks not applied: ${(e as Error).message}`);
      marks.length = 0;
    }
  }
  const settled = { approved: 0, rejected: 0 };
  /** Priorities the owner marked in this run: an approval never overwrites them. */
  const taken = new Set(marks.map((e) => `${e.minutesId}#${e.idx}`));
  for (const v of verdicts) {
    try {
      const res = await deps.decide(v.id, v.status);
      if (res.commit.error) io.error.write(`Commit skipped: ${res.commit.error}\n`);
      if (res.status === "stale") {
        err(`Decision ${v.id} not applied: the target changed since the proposal, so it is now stale.`);
        continue;
      }
      settled[v.status]++;
      out(`Decision ${v.id} ${res.status}${res.minutesId ? ` (${res.minutesId})` : ""}.`);
      const action = actionLine(res);
      if (action) out(action);
      const links = v.links.filter((e) => !taken.has(`${e.minutesId}#${e.idx}`));
      if (links.length) {
        try {
          const commit = await deps.applyMarks(links);
          if (commit.error) io.error.write(`Commit skipped: ${commit.error}\n`);
          for (const e of links) taken.add(`${e.minutesId}#${e.idx}`);
          out(`Marked accepted: ${links.map((e) => `#${e.idx}`).join(", ")} (${links[0]!.minutesId}).`);
        } catch (e) {
          err(`Linked priorities of decision ${v.id} not marked: ${(e as Error).message}`);
        }
      }
    } catch (e) {
      err(`Decision ${v.id}: ${(e as Error).message}`);
    }
  }
  out(`Marked ${marks.length}, approved ${settled.approved}, rejected ${settled.rejected}, skipped ${skipped}.`);
  return code;
}
