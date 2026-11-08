import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import {
  AUDIT_WINDOW,
  READY_FAILURE_MEETINGS,
  READY_REJECTED_PRIORITIES,
  REPAIR_BASELINE_MEETINGS,
  audit,
  buildAudit,
  consumedBy,
  failureClasses,
  readyToReflect,
  repairBaseline,
  type AuditInput,
  type MarkedMinutes,
  type ReflectionLine,
} from "./audit.ts";
import type { ClaudeCall, ClaudeCaller } from "./claude.ts";
import { getPool } from "./db.ts";
import { storeMinutes } from "./minutes.ts";
import { testDb } from "./test-db.ts";
import { roleAnswerSchema } from "./meeting.ts";
import { buildLogEntry, readMeetingLog, type AgentLogEntry, type MeetingLogEntry } from "./meeting-log.ts";
import { makeItem } from "./memory.ts";
import { replayTable } from "./proposal.ts";
import { Budget, COST_RULE, REPLAY_BUDGET_USD, replay, replayCases, type ReplayReport } from "./replay.ts";
import type { Owner, Product } from "./registry.ts";
import type { RoleDef } from "./roles.ts";
import type { Minutes, UsageRow } from "./types.ts";

const product: Product = { id: "demo", name: "Demo", paths: ["/nowhere"], status: "active", roles: ["pm", "dev"] };

const agent = (o: Partial<AgentLogEntry> = {}): AgentLogEntry => ({
  prompt_hash: "p",
  memory_hash: "m",
  model: "sonnet",
  cost_usd: 1,
  duration_ms: 1000,
  error: null,
  priorities: 2,
  sources: 3,
  unresolved_sources: [],
  sin_datos: 0,
  ...o,
});

const meeting = (n: number, o: { agents?: Record<string, AgentLogEntry>; synth?: Partial<MeetingLogEntry["synthesis"]>; total?: number } = {}): MeetingLogEntry => ({
  minutes_id: `m${n}`,
  created_at: new Date(Date.UTC(2026, 9, 1, n)).toISOString(),
  question: "q",
  scope: null,
  brief_hash: null,
  agents: o.agents ?? { pm: agent(), dev: agent() },
  synthesis: {
    first_check: { passed: true, failures: [] },
    repaired: false,
    final_check: { passed: true, failures: [] },
    invented_sources: [],
    cost_usd: 1,
    ...o.synth,
  },
  total_cost_usd: o.total ?? 3,
});

const input = (o: Partial<AuditInput> = {}): AuditInput => ({
  product,
  rosterFailures: [],
  brief: null,
  log: [],
  minutes: [],
  memory: {},
  reflections: [],
  now: new Date("2026-10-13T12:00:00Z"),
  ...o,
});

test("an empty product reports every section with no data and nothing ready", () => {
  const r = buildAudit(input());
  assert.equal(r.meetings.count, 0);
  assert.equal(r.meetings.first_pass_rate, null);
  assert.deepEqual(r.marks, {});
  assert.deepEqual(r.reflections, { last: null, pending_proposals: 0, reverted: 0 });
  assert.deepEqual(r.ready_to_reflect, []);
  assert.equal(r.config.valid, true);
  assert.deepEqual(r.meetings.agents.map((a) => a.agent), ["pm", "dev"]);
});

test("meeting section: rates, per-agent stats, cost share and the last 20 meetings only", () => {
  const log = Array.from({ length: 25 }, (_, i) => meeting(i));
  assert.equal(buildAudit(input({ log })).meetings.count, AUDIT_WINDOW);

  const r = buildAudit(
    input({
      log: [
        meeting(1, { agents: { pm: agent({ error: "claude timed out after 600s", cost_usd: 0 }), dev: agent({ unresolved_sources: [{ type: "path", ref: "x" }], sin_datos: 3, cost_usd: 3 }) } }),
        meeting(2, { synth: { first_check: { passed: false, failures: ["x"] }, repaired: true, invented_sources: [{ type: "path", ref: "y" }], cost_usd: 2 }, total: 4 }),
      ],
    }),
  );
  assert.equal(r.meetings.first_pass_rate, 0.5);
  assert.equal(r.meetings.repair_rate, 0.5);
  assert.equal(r.meetings.invented_sources, 1);
  assert.equal(r.meetings.synthesis_cost_share, 3 / 7);
  const pm = r.meetings.agents.find((a) => a.agent === "pm")!;
  assert.deepEqual([pm.meetings, pm.answers, pm.errors, pm.timeouts, pm.mean_cost_usd], [2, 1, 1, 1, 0.5]);
  const dev = r.meetings.agents.find((a) => a.agent === "dev")!;
  assert.deepEqual([dev.unresolved_sources_per_meeting, dev.sin_datos_per_meeting, dev.mean_cost_usd], [0.5, 1.5, 2]);
});

test("repairs: the first minutes are the baseline, later ones are compared, minutes without check do not count", () => {
  const m = (id: string, day: number, repaired?: boolean): MarkedMinutes => ({
    id,
    created_at: `2026-10-${String(day).padStart(2, "0")}T00:00:00Z`,
    priorities: [],
    ...(repaired === undefined ? {} : { check: { repaired } }),
  });
  assert.equal(REPAIR_BASELINE_MEETINGS, 2);
  const r = buildAudit(input({ minutes: [m("d", 14, false), m("a", 11, true), m("x", 12), m("b", 13, false), m("c", 15, true), m("e", 16, false)] }));
  assert.deepEqual(r.repairs.baseline, { minutes: ["a", "b"], repaired: 1, per_meeting: 0.5 });
  assert.deepEqual(r.repairs.since, { minutes: ["d", "c", "e"], repaired: 1, per_meeting: 1 / 3 });

  assert.deepEqual(repairBaseline([m("a", 11, true)]), {
    baseline: { minutes: ["a"], repaired: 1, per_meeting: 1 },
    since: { minutes: [], repaired: 0, per_meeting: null },
  });
  assert.deepEqual(buildAudit(input()).repairs.baseline, { minutes: [], repaired: 0, per_meeting: null });
});

test("marks are counted per owning agent from the minutes", () => {
  const minutes: MarkedMinutes[] = [
    { id: "a", created_at: "2026-10-01T00:00:00Z", priorities: [{ owner_role: "dev.backend", mark: "accepted" }, { owner_role: "pm", mark: null }] },
    { id: "b", created_at: "2026-10-02T00:00:00Z", priorities: [{ owner_role: "dev.backend", mark: "rejected" }, { owner_role: "dev.backend", mark: "changed" }] },
  ];
  const r = buildAudit(input({ minutes }));
  assert.deepEqual(r.marks["dev.backend"], { accepted: 1, changed: 1, rejected: 1, unmarked: 0 });
  assert.deepEqual(r.marks.pm, { accepted: 0, changed: 0, rejected: 0, unmarked: 1 });
});

test("memory section counts active, retired, lessons, never-helpful items and characters", () => {
  const at = (day: number) => new Date(Date.UTC(2026, 8, day)).toISOString();
  const old = makeItem("old fact", { source: "m", created_at: at(1), taken: [] });
  const helped = { ...makeItem("helped", { source: "m", created_at: at(1), taken: [old.id] }), helpful: 2 };
  const lesson = makeItem("a lesson", { kind: "lesson", source: "m", created_at: at(2), taken: [old.id, helped.id] });
  const gone = { ...makeItem("gone", { source: "m", created_at: at(3), taken: [] }), retired_at: at(4), retired_why: "x" };
  const log = Array.from({ length: 10 }, (_, i) => meeting(i));
  const r = buildAudit(input({ log, memory: { pm: [old, helped, lesson, gone] } }));
  const pm = r.memory.find((m) => m.agent === "pm")!;
  assert.deepEqual([pm.active, pm.retired, pm.lessons, pm.never_helpful], [3, 1, 1, 2]);
  assert.ok(pm.chars > 0 && pm.max_chars === 6000);
  assert.equal(buildAudit(input({ log: log.slice(0, 5), memory: { pm: [old] } })).memory[0]!.never_helpful, 0);
});

test("config section carries the roster failures and the objectives past their date", () => {
  const brief = {
    objectives: [
      { id: "o1", text: "late", signal: "s", due: "2026-10-01" },
      { id: "o2", text: "soon", signal: "s", due: "2026-12-01" },
    ],
  } as AuditInput["brief"];
  const r = buildAudit(input({ rosterFailures: ["bad"], brief }));
  assert.equal(r.config.valid, false);
  assert.deepEqual(r.config.failures, ["bad"]);
  assert.deepEqual(r.config.overdue_objectives, [{ id: "o1", due: "2026-10-01", text: "late" }]);
});

test("reflections section reads the last entry, pending proposals and reverts", () => {
  const r = buildAudit(
    input({
      reflections: [
        { id: "r1", created_at: "2026-10-05T00:00:00Z", status: "applied", proposal: { status: "approved" } },
        { id: "r2", created_at: "2026-10-06T00:00:00Z", status: "applied", proposal: { status: "pending" } },
        { reverts: "r1", created_at: "2026-10-07T00:00:00Z" },
      ],
    }),
  );
  assert.deepEqual(r.reflections, { last: { id: "r2", created_at: "2026-10-06T00:00:00Z" }, pending_proposals: 1, reverted: 1 });
});

test("ready to reflect counts each agent's evidence from its last proposal, not from a memory-only reflection or a revert", () => {
  const bad = { dev: agent({ unresolved_sources: [{ type: "path", ref: "x" }] }) };
  const log = [meeting(2, { agents: bad }), meeting(3, { agents: bad }), meeting(4, { agents: bad })];
  const after = new Date(Date.UTC(2026, 9, 9)).toISOString();
  const ready = (reflections: ReflectionLine[]) => buildAudit(input({ log, reflections })).ready_to_reflect.map((a) => a.agent);
  assert.deepEqual(
    ready([
      { id: "r1", created_at: new Date(Date.UTC(2026, 9, 1, 1)).toISOString(), status: "reverted" },
      { id: "r2", reverts: "r1", created_at: new Date(Date.UTC(2026, 9, 1, 5)).toISOString(), status: "applied" },
    ]),
    ["dev"],
  );
  assert.deepEqual(ready([{ id: "r3", created_at: after, status: "applied", proposal: null }]), ["dev"]);
  assert.deepEqual(ready([{ id: "r3", created_at: after, status: "applied", proposal: { agent: "qa", status: "discarded" } }]), ["dev"]);
  assert.deepEqual(ready([{ id: "r3", created_at: after, status: "applied", proposal: { agent: "dev", status: "discarded" } }]), []);
});

test("evidence counts by minutes id: a meeting that landed after the proposal counts even when it started before", () => {
  const bad = { dev: agent({ unresolved_sources: [{ type: "path", ref: "x" }] }) };
  const log = [meeting(2, { agents: bad }), meeting(3, { agents: bad }), meeting(4, { agents: bad }), meeting(5, { agents: bad })];
  const late = new Date(Date.UTC(2026, 9, 9)).toISOString();
  const ready = (consumed: string[]) =>
    buildAudit(input({ log, reflections: [{ id: "r1", created_at: late, status: "applied", consumed, proposal: { agent: "dev", status: "discarded" } }] })).ready_to_reflect;
  assert.deepEqual(ready(["m2", "m3", "m4", "m5"]), []);
  assert.deepEqual(ready(["m2"]), [{ agent: "dev", reason: "failure_class", failure_class: "unresolved_source", count: 3 }]);
  assert.deepEqual(consumedBy([{ id: "r1", created_at: late, consumed: [] }, { reverts: "r1", created_at: late }]), { ids: new Set(), until: null });
});

test("failure classes: unresolved, timeout, sin datos and the synthesis ones charged to pm", () => {
  const entry = meeting(1, {
    agents: {
      pm: agent({ sin_datos: 2 }),
      dev: agent({ unresolved_sources: [{ type: "path", ref: "x" }], error: "Timeout after 600s" }),
      qa: agent({ sin_datos: 1 }),
    },
    synth: {
      invented_sources: [{ type: "url", ref: "u" }],
      first_check: { passed: false, failures: ['Priority #0 "t" objective "o9" must be one of the brief ids: o1.'] },
    },
  });
  assert.deepEqual(failureClasses(entry), {
    pm: ["sin_datos", "invented_source", "missing_objective"],
    dev: ["unresolved_source", "timeout"],
  });
});

test("ready to reflect: the same failure class in 3 meetings, or 3 rejected priorities of one agent", () => {
  assert.equal(READY_FAILURE_MEETINGS, 3);
  assert.equal(READY_REJECTED_PRIORITIES, 3);
  const bad = { dev: agent({ unresolved_sources: [{ type: "path", ref: "x" }] }), pm: agent() };
  const twice = [meeting(1, { agents: bad }), meeting(2, { agents: bad })];
  assert.deepEqual(readyToReflect(twice, []), []);
  const log = [...twice, meeting(3, { agents: bad }), meeting(4)];
  assert.deepEqual(readyToReflect(log, []), [{ agent: "dev", reason: "failure_class", failure_class: "unresolved_source", count: 3 }]);
  assert.deepEqual(readyToReflect(log, [], consumedBy([{ id: "r", created_at: log[2]!.created_at }])), []);

  const rejected = (id: string, day: number, owner: string, n: number): MarkedMinutes => ({
    id,
    created_at: `2026-10-0${day}T00:00:00Z`,
    priorities: Array.from({ length: n }, () => ({ owner_role: owner, mark: "rejected" as const })),
  });
  const minutes = [rejected("a", 1, "qa", 2), rejected("b", 2, "qa", 1), rejected("c", 2, "pm", 2)];
  assert.deepEqual(readyToReflect([], minutes), [{ agent: "qa", reason: "rejected_priorities", failure_class: null, count: 3 }]);
  assert.deepEqual(readyToReflect([], minutes, consumedBy([{ id: "r", created_at: "2026-10-01T12:00:00Z" }])), []);
});

test("audit() reads a data root and never runs claude", () => {
  const dir = mkdtempSync(join(tmpdir(), "agentsi-noclaude-"));
  const marker = join(dir, "called");
  writeFileSync(join(dir, "claude"), `#!/bin/sh\ntouch ${marker}\nexit 1\n`);
  chmodSync(join(dir, "claude"), 0o755);
  const home = join(dir, "data");
  mkdirSync(join(home, "workspace", "demo"), { recursive: true });
  writeFileSync(
    join(home, "agentsi.config.json"),
    JSON.stringify({ products: [{ id: "demo", name: "Demo", paths: [dir], status: "active", roles: ["pm", "dev"] }] }),
  );
  try {
    const script = `import { audit, getProduct } from ${JSON.stringify(join(import.meta.dirname, "index.ts"))}; console.log(JSON.stringify(await audit(getProduct("demo"))));`;
    const r = spawnSync(process.execPath, ["--import", "tsx", "--input-type=module", "-e", script], {
      encoding: "utf8",
      env: { ...process.env, PATH: `${dir}:${process.env.PATH}`, AGENTSI_HOME: home, AGENTSI_DATABASE_URL: "postgres://x:x@127.0.0.1:1/x" },
    });
    assert.equal(r.status, 0, r.stderr);
    const report = JSON.parse(r.stdout);
    assert.equal(report.product, "demo");
    for (const section of ["config", "meetings", "marks", "memory", "reflections", "ready_to_reflect"]) assert.ok(section in report, section);
    assert.ok(report.memory.length > 0);
    assert.equal(existsSync(marker), false);
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
});

// S006: the repair rounds of the last four meetings, and a folder with only minutes, usage and the meeting log.
const s006Dev: RoleDef = { id: "dev", role: "dev", source: "global", model: "sonnet", body: "dev prompt of today" };
const s006Usage: UsageRow = { role: "dev", model: "sonnet", input_tokens: 0, output_tokens: 0, cache_read: 0, cache_write: 0, cost_usd: 0.02, duration_ms: 1 };

interface S006Meeting {
  id: string;
  question: string;
  repaired: boolean;
}

/** A test database with the product's minutes (and their usage) and the meeting log: nothing else. Returns the folder of the product's path. */
async function s006Home(meetings: S006Meeting[]): Promise<{ home: string; close: () => Promise<void> }> {
  const db = await testDb();
  const home = mkdtempSync(join(tmpdir(), "agentsi-s006-"));
  for (const [i, m] of meetings.entries()) {
    const minutes: Minutes = {
      id: m.id,
      product: "demo",
      scope: null,
      question: m.question,
      created_at: new Date(Date.UTC(2026, 9, 1 + i, 12)).toISOString(),
      roles: { dev: { model: "sonnet", answer: "Churn bajó 2% en agosto.", priorities: [], concerns: [], memory_notes: [] } },
      priorities: [],
      disagreements: [],
      decisions: [],
      script: [],
      check: { passed: true, failures: [], first_failures: m.repaired ? ["Priority #0 does not resolve"] : [], repaired: m.repaired },
      status: "valid",
    };
    const entry = buildLogEntry({
      minutes,
      usage: [s006Usage],
      agents: { dev: { prompt: s006Dev.body, memory: "", model: "sonnet", error: null, duration_ms: 1, unresolved_sources: [] } },
      snapshot: "# Snapshot",
    });
    await storeMinutes(minutes, [s006Usage], { entry, snapshot: "" });
    // The old folder had minutes, usage and the log only, so there are no snapshots.
    await getPool().query("delete from snapshots where product = 'demo'");
  }
  return { home, close: () => db.close() };
}

/** The text `agentsi audit --product demo` prints for the stored meetings. */
async function s006Audit(_home: string): Promise<string> {
  const { renderAudit } = await import("../../cli/src/commands/audit.ts");
  return renderAudit(await audit({ id: "demo", name: "Demo", paths: [_home], status: "active", roles: ["pm", "dev"] }));
}

/** The number n alone in the text, not part of a longer number, a decimal or a percentage. */
const hasCount = (text: string, n: number): boolean => new RegExp(`(?<![\\d.,])${n}(?![\\d.,%])`).test(text);

// S006 criterion 2: the audit prints the repair rounds per meeting for exactly the last four meetings, whatever their topic.
test("S006: agentsi audit prints the repair rounds of the last four meetings, single-topic or not", async () => {
  const meetings: S006Meeting[] = [
    { id: "2026-10-01-1200-churn", question: "Cómo va el churn?", repaired: true },
    { id: "2026-10-02-1200-hosting", question: "Cuánto cuesta el hosting?", repaired: true },
    { id: "2026-10-03-1200-onboarding", question: "Qué pasó con el onboarding?", repaired: false },
    { id: "2026-10-04-1200-multi", question: "Churn, hosting y onboarding: qué priorizo?", repaired: true },
    { id: "2026-10-05-1200-usuarios", question: "Cuántos usuarios activos hay?", repaired: false },
    { id: "2026-10-06-1200-cierre", question: "Qué sigue el mes que viene?", repaired: true },
  ];
  const h = await s006Home(meetings);
  const home = h.home;
  try {
    const out = await s006Audit(home);
    const lines = out.split("\n");
    for (const m of meetings.slice(-4)) {
      const rounds = m.repaired ? 1 : 0;
      assert.ok(
        lines.some((l) => l.includes(m.id) && hasCount(l.replace(m.id, ""), rounds)),
        `the repair rounds of ${m.id} (${rounds}) are missing from:\n${out}`,
      );
    }
    for (const m of meetings.slice(0, -4)) assert.equal(out.includes(m.id), false, `${m.id} is not one of the last four meetings:\n${out}`);
  } finally {
    rmSync(home, { recursive: true, force: true });
    await h.close();
  }
});

// S006 criterion 3: with fewer than four meetings the repair rounds line says [no data].
test("S006: with fewer than four meetings the repair rounds line says [no data]", async () => {
  const meetings: S006Meeting[] = [
    { id: "2026-10-01-1200-churn", question: "Cómo va el churn?", repaired: true },
    { id: "2026-10-02-1200-hosting", question: "Cuánto cuesta el hosting?", repaired: false },
    { id: "2026-10-03-1200-onboarding", question: "Qué pasó con el onboarding?", repaired: false },
  ];
  const h = await s006Home(meetings);
  const home = h.home;
  try {
    const out = await s006Audit(home);
    const line = out.split("\n").find((l) => /ronda/i.test(l));
    assert.ok(line, `no repair rounds line in:\n${out}`);
    assert.match(line, /\[no data\]/);
  } finally {
    rmSync(home, { recursive: true, force: true });
    await h.close();
  }
});

// S006 criterion 4: the replay report and the audit finish and print their numbers from minutes, usage and the meeting log alone.
test("S006: a folder with only minutes, usage and the meeting log gives the replay report and the audit", async () => {
  const meetings: S006Meeting[] = [
    { id: "2026-10-01-1200-churn", question: "Cómo va el churn?", repaired: true },
    { id: "2026-10-02-1200-hosting", question: "Cuánto cuesta el hosting?", repaired: false },
    { id: "2026-10-03-1200-onboarding", question: "Qué pasó con el onboarding?", repaired: true },
    { id: "2026-10-04-1200-multi", question: "Churn, hosting y onboarding: qué priorizo?", repaired: false },
  ];
  const h = await s006Home(meetings);
  const home = h.home;
  try {
    // The database holds the minutes with their usage and the meeting log of each meeting, and no snapshot.
    assert.equal((await getPool().query("select 1 from minutes")).rows.length, meetings.length);
    assert.equal((await getPool().query("select 1 from meeting_log")).rows.length, meetings.length);
    assert.equal((await getPool().query("select 1 from snapshots")).rows.length, 0);

    const audit = await s006Audit(home);
    assert.match(audit, /\d+%/, "the audit prints its numbers");

    // The replay report on the same folder: each meeting is a frozen case, and the candidate's run prints its numbers.
    const product: Product = { id: "demo", name: "Demo", paths: [home], status: "active", roles: ["dev"] };
    const owner: Owner = { name: "Ana", language: "Spanish" };
    const usage = { model: "sonnet", input_tokens: 0, output_tokens: 0, cache_read: 0, cache_write: 0, cost_usd: 0.02, duration_ms: 1 };
    const fake: ClaudeCaller = async (c: ClaudeCall) => ({
      output:
        c.schema === roleAnswerSchema
          ? { answer: "Churn bajó 2% en agosto.", priorities: [], concerns: [], memory_notes: [] }
          : { priorities: [], disagreements: [], decisions: [], script: [] },
      usage,
    });
    const cases = await replayCases(home, "demo", "dev", await readMeetingLog("demo", home));
    assert.equal(cases.length, meetings.length, "each meeting of the folder is a replay case");

    const candidate: RoleDef = { ...s006Dev, body: "dev prompt proposed" };
    const run = await replay(
      { product, agent: "dev", candidate, current: s006Dev, pm: s006Dev, memory: "", brief: null, owner, heldOut: cases, budget: new Budget(REPLAY_BUDGET_USD) },
      { call: fake, check: async () => ({ passed: true, failures: [] }), resolve: async () => true },
    );
    assert.equal(run.aborted, null);
    const report: ReplayReport = {
      tuning: [],
      held_out: cases.map((c) => c.minutes_id),
      rows: run.rows,
      totals: run.totals,
      verdict: run.verdict,
      cost_rule: COST_RULE,
      cost_usd: 0,
      budget_usd: REPLAY_BUDGET_USD,
      aborted: run.aborted,
    };
    const lines = replayTable(report);
    for (const c of cases) assert.ok(lines.some((l) => l.startsWith(`- ${c.minutes_id}:`)), `no replay line for ${c.minutes_id} in:\n${lines.join("\n")}`);
    assert.match(lines[lines.length - 1]!, /^Totales: .*\d/, "the replay totals print their numbers");
  } finally {
    rmSync(home, { recursive: true, force: true });
    await h.close();
  }
});
