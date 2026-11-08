import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { ClaudeCall } from "./claude.ts";
import { getPool } from "./db.ts";
import { git } from "./exec.ts";
import { testDb } from "./test-db.ts";
import type { MeetingLogEntry } from "./meeting-log.ts";
import { activeItems, appendFacts, readMemoryItems, writeMemoryItems, type MemoryItem } from "./memory.ts";
import {
  DELTA_SCHEMA,
  canRevert,
  MAX_ADDS,
  MAX_RETIRES,
  collectSignals,
  curationPrompt,
  readReflections,
  reflect,
  revertReflection,
  unifiedDiff,
  validateDeltas,
  writeReflections,
  type MinutesEvidence,
  type ReflectionEntry,
  type Signal,
} from "./reflect.ts";
import { settleReflection } from "./reflection-decision.ts";
import type { Product } from "./registry.ts";

const item = (id: string, text: string, extra: Partial<MemoryItem> = {}): MemoryItem => ({
  id,
  text,
  kind: "fact",
  helpful: 0,
  harmful: 0,
  source: "migration",
  created_at: "2026-10-01T00:00:00.000Z",
  retired_at: null,
  retired_why: null,
  ...extra,
});

function logEntry(id: string, created_at: string, over: Partial<MeetingLogEntry["synthesis"]> = {}, agents: MeetingLogEntry["agents"] = {}): MeetingLogEntry {
  return {
    minutes_id: id,
    created_at,
    question: "q",
    scope: null,
    brief_hash: null,
    agents,
    synthesis: {
      first_check: { passed: true, failures: [] },
      repaired: false,
      final_check: { passed: true, failures: [] },
      invented_sources: [],
      cost_usd: 0,
      ...over,
    },
    total_cost_usd: 0,
  };
}

const agentLog = (o: Partial<MeetingLogEntry["agents"][string]> = {}): MeetingLogEntry["agents"][string] => ({
  prompt_hash: "",
  memory_hash: "",
  model: "haiku",
  cost_usd: 0,
  duration_ms: 0,
  error: null,
  priorities: 1,
  sources: 1,
  unresolved_sources: [],
  sin_datos: 0,
  ...o,
});

const minutes = (id: string, marks: [string, "accepted" | "changed" | "rejected" | null][], rejected: string[] = []): MinutesEvidence => ({
  id,
  created_at: "2026-10-02T00:00:00.000Z",
  priorities: marks.map(([owner_role, mark], i) => ({ title: `p${i}`, why: "", sources: [], owner_role, mark, is_new: false })),
  decisions: rejected.map((text) => ({ text, why: "", status: "rejected" as const })),
});

const entry = (id: string, created_at: string, extra: Partial<ReflectionEntry> = {}): ReflectionEntry => ({
  id,
  product: "demo",
  created_at,
  trigger: "manual",
  signals: [],
  memory: [],
  proposal: null,
  status: "applied",
  commit: null,
  ...extra,
});

test("collectSignals reads failures from the meeting log and credits the synthesis failures to pm", () => {
  const log = [
    logEntry(
      "m1",
      "2026-10-02T00:00:00.000Z",
      { first_check: { passed: false, failures: ["priority 0 cites nothing"] }, invented_sources: [{ type: "path", ref: "x.md" }] },
      {
        dev: agentLog({ unresolved_sources: [{ type: "path", ref: "docs/nope.md" }], sin_datos: 2 }),
        qa: agentLog({ error: "claude timed out after 360s", sin_datos: 1 }),
      },
    ),
  ];
  const signals = collectSignals({ log, minutes: [], reflections: [] });
  assert.deepEqual(signals, [
    { agent: "pm", kind: "first_check_failure", ref: "m1", detail: "priority 0 cites nothing" },
    { agent: "pm", kind: "invented_source", ref: "m1", detail: "x.md" },
    { agent: "dev", kind: "unresolved_source", ref: "m1", detail: "docs/nope.md" },
    { agent: "dev", kind: "sin_datos", ref: "m1", detail: '2 "sin datos"' },
    { agent: "qa", kind: "error", ref: "m1", detail: "claude timed out after 360s" },
  ]);
});

test("collectSignals credits marks to owner_role and rejected decisions to pm", () => {
  const signals = collectSignals({
    log: [],
    minutes: [minutes("m1", [["dev.backend", "accepted"], ["qa", "rejected"], ["dev", null], ["dev", "changed"]], ["drop the web"])],
    reflections: [],
  });
  assert.deepEqual(
    signals.map((s) => [s.agent, s.kind, s.detail]),
    [
      ["dev.backend", "mark_accepted", "#0 p0"],
      ["qa", "mark_rejected", "#1 p1"],
      ["dev", "mark_changed", "#3 p3"],
      ["pm", "decision_rejected", "drop the web"],
    ],
  );
});

test("collectSignals skips meeting lines before the cursor and marks an earlier entry recorded", () => {
  const mark: Signal = { agent: "qa", kind: "mark_rejected", ref: "m1", detail: "#0 p0" };
  const reflections = [entry("r-1", "2026-10-03T00:00:00.000Z", { signals: [mark] })];
  const signals = collectSignals({
    log: [
      logEntry("m1", "2026-10-02T00:00:00.000Z", {}, { dev: agentLog({ error: "boom" }) }),
      logEntry("m2", "2026-10-04T00:00:00.000Z", {}, { dev: agentLog({ error: "boom" }) }),
    ],
    minutes: [minutes("m1", [["qa", "rejected"], ["qa", "accepted"]])],
    reflections,
  });
  assert.deepEqual(signals, [
    { agent: "dev", kind: "error", ref: "m2", detail: "boom" },
    { agent: "qa", kind: "mark_accepted", ref: "m1", detail: "#1 p1" },
  ]);
});

test("collectSignals counts a meeting line by minutes id, so a meeting that landed after a reflection started still counts", () => {
  const reflections = [entry("r-1", "2026-10-03T00:00:00.000Z", { consumed: ["m1"] })];
  const signals = collectSignals({
    log: [
      logEntry("m1", "2026-10-02T00:00:00.000Z", {}, { dev: agentLog({ error: "boom" }) }),
      logEntry("m-late", "2026-10-02T23:59:00.000Z", {}, { dev: agentLog({ error: "late" }) }),
    ],
    minutes: [],
    reflections,
  });
  assert.deepEqual(signals, [{ agent: "dev", kind: "error", ref: "m-late", detail: "late" }]);
});

test("collectSignals turns a revert after the cursor into a signal for each agent it touched, and the revert does not move the cursor", () => {
  const change = { agent: "dev", added: ["m-1"], retired: [], helpful: [], harmful: [], dropped: [], diff: "" };
  const reflections = [
    entry("r-1", "2026-10-03T00:00:00.000Z", { memory: [change], status: "reverted" }),
    entry("r-2", "2026-10-05T00:00:00.000Z", { reverts: "r-1" }),
  ];
  const signals = collectSignals({
    log: [logEntry("m2", "2026-10-04T00:00:00.000Z", {}, { qa: agentLog({ error: "x" }) })],
    minutes: [],
    reflections,
  });
  assert.deepEqual(signals, [
    { agent: "qa", kind: "error", ref: "m2", detail: "x" },
    { agent: "dev", kind: "reverted_reflection", ref: "r-1", detail: "reverted by r-2" },
  ]);
});

test("only an applied entry with a memory diff can be reverted", () => {
  const memory = [{ agent: "dev", added: [], retired: [], helpful: [], harmful: [], dropped: [], diff: "+x" }];
  assert.equal(canRevert(entry("r-1", "2026-10-03T00:00:00.000Z", { memory })), true);
  assert.equal(canRevert(entry("r-1", "2026-10-03T00:00:00.000Z", { memory, status: "reverted" })), false);
  assert.equal(canRevert(entry("r-2", "2026-10-03T00:00:00.000Z", { memory, reverts: "r-1" })), false);
  assert.equal(canRevert(entry("r-1", "2026-10-03T00:00:00.000Z", { memory: [{ ...memory[0]!, diff: "" }] })), false);
});

test("validateDeltas drops adds without signal evidence, unknown ids and anything over the caps", () => {
  const items = [item("m-a", "uno"), item("m-b", "dos"), item("m-old", "viejo", { retired_at: "2026-10-01T00:00:00.000Z", retired_why: "x" })];
  const signals: Signal[] = [{ agent: "dev", kind: "mark_rejected", ref: "m1", detail: "#0 p0" }];
  const { deltas, dropped } = validateDeltas(
    {
      add: [
        { text: "  cita el plan  ", kind: "lesson", evidence: ["m1", "made-up"] },
        { text: "sin evidencia", kind: "fact", evidence: [] },
        { text: "evidencia inventada", kind: "fact", evidence: ["m9"] },
        { text: "uno", kind: "fact", evidence: ["m1"] },
      ],
      retire: [{ id: "m-a", why: "stale" }, { id: "m-old", why: "x" }, { id: "m-zz", why: "x" }, { id: "m-b", why: "" }],
      helpful: ["m-b", "m-b", "m-zz"],
      harmful: ["m-old"],
    },
    items,
    signals,
  );
  assert.deepEqual(deltas, { add: [{ text: "cita el plan", evidence: ["m1"] }], retire: [{ id: "m-a", why: "stale" }], helpful: ["m-b"], harmful: [] });
  assert.equal(dropped.length, 8);
  assert.ok(dropped.includes('add "sin evidencia": cites no signal'));
  assert.ok(dropped.includes('add "evidencia inventada": cites no signal'));

  const many = Array.from({ length: 12 }, (_, n) => item(`m-${n}`, `t${n}`));
  const capped = validateDeltas(
    {
      add: Array.from({ length: MAX_ADDS + 1 }, (_, n) => ({ text: `nuevo ${n}`, kind: "lesson", evidence: ["m1"] })),
      retire: many.map((i) => ({ id: i.id, why: "old" })),
      helpful: [],
      harmful: [],
    },
    many,
    signals,
  );
  assert.equal(capped.deltas.add.length, MAX_ADDS);
  assert.equal(capped.deltas.retire.length, MAX_RETIRES);
  assert.equal(capped.dropped.length, 1 + 12 - MAX_RETIRES);
});

test("validateDeltas survives a malformed answer", () => {
  assert.deepEqual(validateDeltas("nonsense", [], []).deltas, { add: [], retire: [], helpful: [], harmful: [] });
  assert.deepEqual(validateDeltas({ add: [null, 3], retire: "x" }, [], []).dropped, []);
});

test("the curation prompt holds the brief, the active items as id lines and the signals", () => {
  const prompt = curationPrompt({
    agent: "dev",
    items: [item("m-a", "uno", { helpful: 2 }), item("m-old", "viejo", { retired_at: "2026-10-01T00:00:00.000Z", retired_why: "x" })],
    signals: [{ agent: "dev", kind: "mark_rejected", ref: "m1", detail: "#0 p0" }],
    briefXml: "<brief>b</brief>",
  });
  assert.equal(prompt, "<brief>b</brief>\n\n<agent>dev</agent>\n<items>\n[m-a] (fact, helpful 2, harmful 0) uno\n</items>\n<signals>\n- ref m1 · mark_rejected · #0 p0\n</signals>");
});

test("unifiedDiff is empty for equal texts and shows added and removed lines", async () => {
  assert.equal(await unifiedDiff("a", "a", "x"), "");
  const diff = await unifiedDiff("- [m-1] uno\n- [m-2] dos", "- [m-3] tres\n- [m-1] uno", "demo/memory/dev");
  assert.match(diff, /^--- a\/demo\/memory\/dev\n\+\+\+ b\/demo\/memory\/dev\n@@/);
  assert.match(diff, /^\+- \[m-3\] tres$/m);
  assert.match(diff, /^-- \[m-2\] dos$/m);
});

const PRODUCT: Product = { id: "demo", name: "Demo", paths: ["/p"], status: "active", roles: ["pm", "dev", "qa"] };

/** A meeting as the database holds it: the minutes row with its JSON, and the meeting log line. */
async function saveMeeting(log: { minutes_id: string }, m: { id: string }): Promise<void> {
  await getPool().query("insert into minutes (id, product, data) values ($1, 'demo', $2::jsonb)", [m.id, JSON.stringify(m)]);
  await getPool().query("insert into meeting_log (product, minutes_id, entry) values ('demo', $1, $2::jsonb)", [log.minutes_id, JSON.stringify(log)]);
}

async function fixture() {
  const db = await testDb();
  const root = mkdtempSync(join(tmpdir(), "agentsi-reflect-"));
  const ws = join(root, "workspace/demo");
  mkdirSync(join(ws, "minutes"), { recursive: true });
  await writeMemoryItems("demo", "dev", [item("m-aaaa", "El plan vive en docs/plans."), item("m-bbbb", "Pendiente a 2026-10-01: cerrar U3.")]);
  await writeMemoryItems("demo", "qa", [item("m-cccc", "Los tests corren con node --test.")]);
  const log = logEntry("2026-10-02-1200-q", "2026-10-02T12:00:00.000Z", {}, { dev: agentLog({ unresolved_sources: [{ type: "path", ref: "docs/nope.md" }] }) });
  await saveMeeting(log, minutes("2026-10-02-1200-q", [["dev", "rejected"]]));
  await git(root, "init", "-q");
  await git(root, "config", "user.email", "t@example.com");
  await git(root, "config", "user.name", "T");
  await git(root, "add", ".");
  await git(root, "commit", "-qm", "init");
  return {
    root,
    ws,
    cleanup: async () => {
      rmSync(root, { recursive: true, force: true });
      await db.close();
    },
  };
}

const count = async (root: string) => Number((await git(root, "rev-list", "--count", "HEAD")).stdout.trim());

function stub(answers: Record<string, unknown>) {
  const calls: ClaudeCall[] = [];
  const call = async (c: ClaudeCall) => {
    calls.push(c);
    const agent = /<agent>(.+)<\/agent>/.exec(c.prompt)![1]!;
    return {
      output: answers[agent] ?? { add: [], retire: [], helpful: [], harmful: [] },
      usage: { model: "haiku", input_tokens: 0, output_tokens: 0, cache_read: 0, cache_write: 0, cost_usd: 0.01, duration_ms: 1 },
    };
  };
  return { calls, call };
}

const quiet = () => {};

test("reflect writes one entry and no commit, applies valid deltas and drops the ones that cite no signal", async () => {
  const f = await fixture();
  try {
    const s = stub({
      dev: {
        add: [
          { text: "Citá solo rutas que existen.", kind: "fact", evidence: ["2026-10-02-1200-q"] },
          { text: "Una idea sin respaldo.", kind: "fact", evidence: ["somewhere"] },
        ],
        retire: [{ id: "m-bbbb", why: "estado viejo" }],
        helpful: ["m-aaaa"],
        harmful: [],
      },
    });
    const before = await count(f.root);
    const now = new Date("2026-10-13T15:30:00.000Z");
    const { entry, commit } = await reflect(PRODUCT, { root: f.root, now, call: s.call, warn: quiet });

    assert.equal(commit.committed, false);
    assert.equal(await count(f.root), before, "the data root gets no commit");
    assert.equal((await git(f.root, "status", "--porcelain")).stdout.trim(), "");
    assert.deepEqual(await readReflections("demo", f.root), [entry]);

    const agentOf = (c: ClaudeCall) => c.prompt.match(/<agent>(.+)<\/agent>/)![1];
    assert.deepEqual(s.calls.map(agentOf).sort(), ["dev", "qa"], "before the first reflection every item is new");
    const c = s.calls.find((x) => agentOf(x) === "dev")!;
    assert.equal(c.model, "haiku");
    assert.equal(c.effort, "low");
    assert.deepEqual(c.tools, []);
    assert.equal(c.schema, DELTA_SCHEMA);
    const stored = JSON.stringify(await readMemoryItems("demo", "dev"), null, 2);
    assert.ok(!c.prompt.includes(stored), "the prompt never holds the stored memory as JSON");
    assert.doesNotMatch(c.prompt, /"items"|retired_at|"helpful":/);
    assert.match(c.prompt, /^\[m-aaaa\] \(fact, helpful 0, harmful 0\) El plan vive en docs\/plans\.$/m);

    const items = await readMemoryItems("demo", "dev");
    const added = items.filter((i) => i.source === entry!.id);
    assert.deepEqual(added.map((i) => [i.text, i.kind]), [["Citá solo rutas que existen.", "lesson"]]);
    assert.equal(items.find((i) => i.id === "m-bbbb")!.retired_why, "estado viejo");
    assert.equal(items.find((i) => i.id === "m-aaaa")!.helpful, 1);

    assert.deepEqual(entry!.memory.map((m) => m.agent), ["dev", "qa"]);
    assert.equal(entry!.memory[1]!.diff, "");
    const change = entry!.memory[0]!;
    assert.deepEqual(change.added, [added[0]!.id]);
    assert.deepEqual(change.retired, ["m-bbbb"]);
    assert.deepEqual(change.dropped, ['add "Una idea sin respaldo.": cites no signal']);
    assert.match(change.diff, /^\+- \[m-[0-9a-f]+\] Citá solo rutas que existen\.$/m);
    assert.match(change.diff, /^-- \[m-bbbb\] Pendiente/m);
    assert.deepEqual(
      entry!.signals.map((x) => x.kind),
      ["unresolved_source", "mark_rejected"],
    );
    assert.equal(entry!.proposal, null);
    assert.equal(entry!.status, "applied");
  } finally {
    await f.cleanup();
  }
});

test("facts a meeting adds during the curation calls survive, and deltas on items gone meanwhile are skipped", async () => {
  const f = await fixture();
  try {
    const s = stub({
      dev: {
        add: [{ text: "Citá solo rutas que existen.", kind: "lesson", evidence: ["2026-10-02-1200-q"] }],
        retire: [{ id: "m-bbbb", why: "estado viejo" }],
        helpful: ["m-aaaa"],
        harmful: [],
      },
    });
    const call = async (c: ClaudeCall) => {
      if (c.prompt.includes("<agent>dev</agent>")) {
        await appendFacts("demo", "dev", ["Un hecho de una reunión que terminó durante la reflexión."], "2026-10-13-1500-late", new Date("2026-10-13T15:00:00.000Z"));
        await writeMemoryItems("demo", "dev", (await readMemoryItems("demo", "dev")).filter((i) => i.id !== "m-bbbb"));
      }
      return s.call(c);
    };
    const { entry } = await reflect(PRODUCT, { root: f.root, now: new Date("2026-10-13T15:30:00.000Z"), call, warn: quiet });

    const items = await readMemoryItems("demo", "dev");
    assert.ok(items.some((i) => i.source === "2026-10-13-1500-late"), "the meeting's fact is kept");
    assert.ok(items.some((i) => i.text === "Citá solo rutas que existen."));
    assert.equal(items.find((i) => i.id === "m-aaaa")!.helpful, 1);
    const change = entry!.memory.find((m) => m.agent === "dev")!;
    assert.deepEqual(change.retired, []);
    assert.ok(change.dropped.includes("retire m-bbbb: no longer active"));
    assert.doesNotMatch(change.diff, /^\+.*Un hecho de una reunión/m, "the diff adds only this reflection's items");
  } finally {
    await f.cleanup();
  }
});

test("a proposal settled while reflect waits on its calls keeps its status", async () => {
  const f = await fixture();
  try {
    const earlier = entry("r-0", "2026-10-01T00:00:00.000Z", { proposal: { agent: "dev", status: "pending", decision_id: 3 } as ReflectionEntry["proposal"] });
    await writeReflections("demo", [earlier], f.root);
    const s = stub({});
    let settled = false;
    const call = async (c: ClaudeCall) => {
      if (!settled) {
        settled = true;
        await settleReflection("demo", "r-0", "rejected", { root: f.root });
      }
      return s.call(c);
    };
    await reflect(PRODUCT, { root: f.root, now: new Date("2026-10-13T15:30:00.000Z"), call, warn: quiet, proposals: false });
    const lines = await readReflections("demo", f.root);
    assert.deepEqual(lines.map((e) => [e.id, e.proposal?.status ?? null]), [["r-0", "rejected"], [lines[1]!.id, null]]);
  } finally {
    await f.cleanup();
  }
});

test("reflect records what it consumed, and a meeting started before it but logged after counts in the next one", async () => {
  const f = await fixture();
  try {
    const { entry } = await reflect(PRODUCT, { root: f.root, now: new Date("2026-10-13T15:30:00.000Z"), call: stub({}).call, warn: quiet });
    assert.deepEqual(entry!.consumed, ["2026-10-02-1200-q", "migration"]);

    const late = logEntry("2026-10-13-1500-late", "2026-10-13T15:00:00.000Z", {}, { qa: agentLog({ error: "boom" }) });
    await getPool().query("insert into meeting_log (product, minutes_id, entry) values ('demo', $1, $2::jsonb)", [late.minutes_id, JSON.stringify(late)]);
    await appendFacts("demo", "qa", ["Un hecho tardío."], late.minutes_id, new Date(late.created_at));

    const s = stub({});
    const next = await reflect(PRODUCT, { root: f.root, now: new Date("2026-10-13T16:00:00.000Z"), call: s.call, warn: quiet });
    assert.deepEqual(next.entry!.signals, [{ agent: "qa", kind: "error", ref: late.minutes_id, detail: "boom" }]);
    assert.deepEqual(next.entry!.consumed, [late.minutes_id]);
    assert.equal(s.calls.length, 1);
  } finally {
    await f.cleanup();
  }
});

test("reflect revert refuses an entry that changed no memory", async () => {
  const f = await fixture();
  try {
    const { entry } = await reflect(PRODUCT, { root: f.root, now: new Date("2026-10-13T15:30:00.000Z"), call: stub({}).call, warn: quiet });
    await assert.rejects(revertReflection(PRODUCT, entry!.id, { root: f.root }), /changed no memory/);
  } finally {
    await f.cleanup();
  }
});

test("readReflections returns the stored entries in order, and warns about a row that is not JSON text", async () => {
  const f = await fixture();
  try {
    const good = entry("r-1", "2026-10-03T00:00:00.000Z");
    const second = entry("r-2", "2026-10-04T00:00:00.000Z");
    await writeReflections("demo", [good, second], f.root);
    const warnings: string[] = [];
    assert.deepEqual(await readReflections("demo", f.root, (m) => warnings.push(m)), [good, second]);
    assert.deepEqual(warnings, []);
  } finally {
    await f.cleanup();
  }
});

test("a second reflect without new evidence writes nothing and calls no model", async () => {
  const f = await fixture();
  try {
    await reflect(PRODUCT, { root: f.root, now: new Date("2026-10-13T15:30:00.000Z"), call: stub({}).call, warn: quiet });
    const before = await count(f.root);
    const s = stub({});
    const res = await reflect(PRODUCT, { root: f.root, now: new Date("2026-10-13T16:00:00.000Z"), call: s.call, warn: quiet });
    assert.equal(res.entry, null);
    assert.equal(s.calls.length, 0);
    assert.equal(await count(f.root), before);
  } finally {
    await f.cleanup();
  }
});

test("a failed curation call aborts reflect before any write", async () => {
  const f = await fixture();
  try {
    const before = await count(f.root);
    await assert.rejects(reflect(PRODUCT, { root: f.root, call: async () => Promise.reject(new Error("down")), warn: quiet }), /down/);
    assert.equal(await count(f.root), before);
    assert.deepEqual(await readReflections("demo", f.root), []);
  } finally {
    await f.cleanup();
  }
});

test("reflect revert restores the previous items, bumps harmful on the additions, keeps later counters and refuses twice", async () => {
  const f = await fixture();
  try {
    const s = stub({
      dev: {
        add: [{ text: "Citá solo rutas que existen.", kind: "lesson", evidence: ["2026-10-02-1200-q"] }],
        retire: [{ id: "m-bbbb", why: "estado viejo" }],
        helpful: ["m-aaaa"],
        harmful: [],
      },
    });
    const original = await readMemoryItems("demo", "dev");
    const { entry } = await reflect(PRODUCT, { root: f.root, now: new Date("2026-10-13T15:30:00.000Z"), call: s.call, warn: quiet });
    const addedId = entry!.memory[0]!.added[0]!;

    const later = (await readMemoryItems("demo", "dev")).map((i) => (i.id === "m-aaaa" ? { ...i, helpful: i.helpful + 1 } : i));
    await writeMemoryItems("demo", "dev", later);

    const before = await count(f.root);
    const res = await revertReflection(PRODUCT, entry!.id, { root: f.root, now: new Date("2026-10-13T17:00:00.000Z") });
    assert.equal(res.commit.committed, false);
    assert.equal(await count(f.root), before, "the data root gets no commit");

    const items = await readMemoryItems("demo", "dev");
    assert.deepEqual(activeItems(items).map((i) => [i.id, i.text]), activeItems(original).map((i) => [i.id, i.text]));
    const added = items.find((i) => i.id === addedId)!;
    assert.equal(added.harmful, 1);
    assert.match(added.retired_why!, /^reverted by r-/);
    assert.equal(items.find((i) => i.id === "m-aaaa")!.helpful, 1, "the later bump survives, the reflected one is undone");
    assert.equal(items.find((i) => i.id === "m-bbbb")!.retired_at, null);

    const lines = await readReflections("demo", f.root);
    assert.equal(lines.length, 2);
    assert.equal(lines[0]!.status, "reverted");
    assert.deepEqual({ reverts: lines[1]!.reverts, status: lines[1]!.status }, { reverts: entry!.id, status: "applied" });
    assert.deepEqual(lines[1]!.memory[0]!.harmful, [addedId]);
    assert.deepEqual(lines[1]!.memory[0]!.restored, ["m-bbbb"]);

    await assert.rejects(revertReflection(PRODUCT, entry!.id, { root: f.root }), /already reverted/);
    await assert.rejects(revertReflection(PRODUCT, lines[1]!.id, { root: f.root }), /is a revert/);
    await assert.rejects(revertReflection(PRODUCT, "r-nope", { root: f.root }), /not found/);
  } finally {
    await f.cleanup();
  }
});
