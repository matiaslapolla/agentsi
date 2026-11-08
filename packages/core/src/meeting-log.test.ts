import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { AgentAnswerLog } from "./meeting-log.ts";
import type { Product } from "./registry.ts";
import type { Minutes, UsageRow } from "./types.ts";

// The data root is read from AGENTSI_HOME when paths.ts loads, so the test home is set before the product code loads.
const base = mkdtempSync(join(tmpdir(), "agentsi-t028-log-"));
const home = join(base, "data");
const repo = join(base, "product");
mkdirSync(repo);
mkdirSync(join(home, "workspace", "demo"), { recursive: true });
writeFileSync(
  join(home, "agentsi.config.json"),
  JSON.stringify({ products: [{ id: "demo", name: "Demo", paths: [repo], status: "active", roles: ["pm", "dev"] }] }, null, 2) + "\n",
);
const gitEnv = { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@t" };
execFileSync("git", ["init", "-q"], { cwd: home });
execFileSync("git", ["add", "."], { cwd: home });
execFileSync("git", ["commit", "-qm", "init"], { cwd: home, env: gitEnv });
process.env.AGENTSI_HOME = home;
process.env.AGENTSI_DATABASE_URL = "postgres://x:x@127.0.0.1:1/x";
Object.assign(process.env, gitEnv);

const { resolveSource } = await import("./check.ts");
const { NO_DATA, appendLedger, buildLogEntry, parseMeetingLog, sha256, readMeetingLog } = await import("./meeting-log.ts");
const { renderMinutesMarkdown } = await import("./minutes-md.ts");
const { writeMinutes } = await import("./minutes.ts");
const { readReflections, updateReflections } = await import("./reflections.ts");
const { archiveMinutes, archivedIds } = await import("./archive.ts");
const { replayCases } = await import("./replay.ts");
const { testDb } = await import("./test-db.ts");

const usageRow = (role: string, cost_usd: number): UsageRow => ({
  role,
  model: "m",
  input_tokens: 1,
  output_tokens: 1,
  cache_read: 0,
  cache_write: 0,
  cost_usd,
  duration_ms: 10,
});

const run = (over: Partial<AgentAnswerLog> = {}): AgentAnswerLog => ({
  prompt: "You are pm.",
  memory: "notes",
  model: "m",
  error: null,
  duration_ms: 1200,
  unresolved_sources: [],
  ...over,
});

const minutes = (): Minutes => ({
  id: "2026-10-13-0900-q",
  product: "fixture",
  scope: null,
  question: "q",
  created_at: "2026-10-13T09:00:00.000Z",
  roles: {
    pm: {
      model: "m",
      answer: "Sin datos de uso. Además SIN DATOS de ventas.",
      priorities: [{ title: "a", why: "w", sources: [{ type: "path", ref: "docs/a.md" }, { type: "commit", ref: "abc1234" }] }],
      concerns: ["sin datos sobre churn"],
      memory_notes: [],
    },
    dev: { model: "m", answer: "", priorities: [], concerns: ["Role call failed: timeout"], memory_notes: [] },
  },
  priorities: [
    {
      title: "a",
      why: "w",
      sources: [
        { type: "path", ref: "docs/a.md" },
        { type: "path", ref: "docs/made-up.md" },
        { type: "commit", ref: "feedbee" },
        { type: "url", ref: "https://x.test/in-snapshot" },
      ],
      owner_role: "pm",
      mark: null,
      is_new: false,
    },
  ],
  disagreements: [],
  decisions: [],
  script: [],
  check: { passed: true, failures: [], first_failures: ["Priority #0: path source \"x\" does not resolve."], repaired: true },
  status: "valid",
});

const usage = [usageRow("pm", 0.1), usageRow("synthesis", 0.2), usageRow("synthesis-repair", 0.3)];

test("buildLogEntry hashes the prompt and the memory, counts sin datos and keeps the agent's unresolved sources", () => {
  const missing = { type: "path" as const, ref: "docs/gone.md" };
  const e = buildLogEntry({
    minutes: minutes(),
    usage,
    agents: { pm: run({ unresolved_sources: [missing] }), dev: run({ error: "timeout", model: "sonnet" }) },
    snapshot: "snapshot https://x.test/in-snapshot",
  });
  assert.equal(e.agents.pm!.prompt_hash, sha256("You are pm."));
  assert.equal(e.agents.pm!.memory_hash, sha256("notes"));
  assert.deepEqual(e.agents.pm!.unresolved_sources, [missing]);
  assert.equal(e.agents.pm!.sin_datos, 3);
  assert.equal(e.agents.pm!.priorities, 1);
  assert.equal(e.agents.pm!.sources, 2);
  assert.equal(e.agents.pm!.cost_usd, 0.1);
  assert.equal(e.brief_hash, null);
});

test("buildLogEntry counts the no-data marker in any case and in any answer language, next to the old phrase", () => {
  const m = minutes();
  m.roles.pm = { ...m.roles.pm!, answer: `${NO_DATA} on usage. Dados de vendas: [NO DATA].`, concerns: ["sin datos sobre churn", "no data here"] };
  const e = buildLogEntry({ minutes: m, usage, agents: { pm: run() }, snapshot: "" });
  assert.equal(NO_DATA, "[no data]");
  assert.equal(e.agents.pm!.sin_datos, 3);
});

test("buildLogEntry marks an errored agent", () => {
  const e = buildLogEntry({ minutes: minutes(), usage, agents: { dev: run({ error: "timeout" }) }, snapshot: "" });
  assert.equal(e.agents.dev!.error, "timeout");
  assert.equal(e.agents.dev!.cost_usd, 0);
  assert.equal(e.agents.dev!.priorities, 0);
});

test("buildLogEntry keeps the first check when the repair passes", () => {
  const e = buildLogEntry({ minutes: minutes(), usage, agents: {}, snapshot: "" });
  assert.equal(e.synthesis.repaired, true);
  assert.deepEqual(e.synthesis.first_check, { passed: false, failures: minutes().check.first_failures });
  assert.deepEqual(e.synthesis.final_check, { passed: true, failures: [] });
});

test("buildLogEntry lists synthesis sources that no agent gave and the snapshot lacks", () => {
  const e = buildLogEntry({ minutes: minutes(), usage, agents: {}, snapshot: "snapshot https://x.test/in-snapshot" });
  assert.deepEqual(e.synthesis.invented_sources, [
    { type: "path", ref: "docs/made-up.md" },
    { type: "commit", ref: "feedbee" },
  ]);
});

test("buildLogEntry sums costs", () => {
  const e = buildLogEntry({ minutes: minutes(), usage, agents: {}, snapshot: "" });
  assert.ok(Math.abs(e.synthesis.cost_usd - 0.5) < 1e-9);
  assert.ok(Math.abs(e.total_cost_usd - 0.6) < 1e-9);
});

test("resolveSource rejects a path that does not exist in the product", async () => {
  const root = mkdtempSync(join(tmpdir(), "agentsi-resolve-"));
  mkdirSync(join(root, "docs"));
  writeFileSync(join(root, "docs/a.md"), "# A\n");
  const product: Product = { id: "fixture", name: "Fixture", paths: [root], status: "active", roles: ["pm"] };
  assert.equal(await resolveSource({ type: "path", ref: "docs/a.md" }, product), true);
  assert.equal(await resolveSource({ type: "path", ref: "docs/gone.md" }, product), false);
  assert.equal(await resolveSource({ type: "url", ref: "ftp://x" }, product), false);
});

test("appendLedger appends one line per meeting and writes the snapshot", () => {
  const dir = mkdtempSync(join(tmpdir(), "agentsi-ledger-"));
  const log = join(dir, "meetings.jsonl");
  const snap = join(dir, "snapshots", "x.md");
  const entry = buildLogEntry({ minutes: minutes(), usage, agents: { pm: run() }, snapshot: "S" });
  appendLedger(log, snap, entry, "# Snapshot\n");
  appendLedger(log, join(dir, "snapshots", "y.md"), { ...entry, minutes_id: "y" }, "# Snapshot 2\n");
  assert.equal(readFileSync(snap, "utf8"), "# Snapshot\n");
  const parsed = parseMeetingLog(readFileSync(log, "utf8"));
  assert.deepEqual(parsed.map((p) => p.minutes_id), [entry.minutes_id, "y"]);
  assert.deepEqual(parsed[0], JSON.parse(JSON.stringify(entry)));
});

test("parseMeetingLog skips broken lines with a warning", () => {
  const entry = buildLogEntry({ minutes: minutes(), usage, agents: { pm: run() }, snapshot: "" });
  const warnings: string[] = [];
  const text = [JSON.stringify(entry), "{not json", '{"minutes_id":1}', "", JSON.stringify(entry)].join("\n");
  assert.equal(parseMeetingLog(text, (w) => warnings.push(w)).length, 2);
  assert.equal(warnings.length, 2);
  assert.match(warnings[0]!, /line 2/);
});

test("the meeting log, listens, reflections, snapshots and archive records live in the database only", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const m = { ...minutes(), product: "demo" };
  const snapshotText = "# Snapshot\nfrozen for the replay\n";
  const entry = buildLogEntry({ minutes: m, usage, agents: { pm: run() }, snapshot: snapshotText });
  await writeMinutes(m, usage, { entry, snapshot: snapshotText });

  // A listen: the owner played the call to the end.
  mkdirSync(join(home, "workspace", "demo", "audio", m.id), { recursive: true });
  writeFileSync(join(home, "workspace", "demo", "audio", m.id, "call.webm"), "audio");
  const web = await import("../../../apps/web/lib/inbox-data.ts");
  assert.equal(await web.recordListen("demo", m.id), true, "the first listen is recorded");
  assert.equal(await web.recordListen("demo", m.id), false, "the second listen of the same call is already stored");

  // A reflection, as `agentsi reflect` writes it.
  const reflection = {
    id: "r-2026-10-30-1",
    product: "demo",
    created_at: "2026-10-30T10:00:00.000Z",
    trigger: "manual" as const,
    signals: [{ agent: "pm", kind: "first_check_failure" as const, ref: m.id, detail: "Priority #0 does not resolve." }],
    memory: [],
    proposal: null,
    consumed: [m.id],
    status: "applied" as const,
    commit: null,
  };
  await updateReflections("demo", (entries) => [...entries, reflection]);

  // An archive record, as `agentsi archive` writes it.
  await archiveMinutes("demo", m.id);

  // The log, reflect, replay, archive and snapshot readers return what was stored.
  const log = await readMeetingLog("demo", home, () => {});
  assert.deepEqual(log, [JSON.parse(JSON.stringify(entry))], "the log returns the stored meeting");
  assert.deepEqual(await readReflections("demo", home, () => {}), [reflection], "reflect reads back the stored reflection");
  assert.deepEqual([...(await archivedIds("demo", home))], [m.id], "archive reads back the archived minutes");
  const cases = await replayCases(home, "demo", "pm", log);
  assert.deepEqual(cases.map((c) => c.minutes_id), [m.id], "replay finds the meeting");
  assert.equal(cases[0]!.snapshot, snapshotText, "the snapshot comes back as it was stored");
  assert.equal(cases[0]!.minutes.question, m.question, "replay reads the minutes from the database");

  // The data root holds none of the old files.
  const ws = join(home, "workspace", "demo");
  const names = (dir: string): string[] =>
    existsSync(dir) ? readdirSync(dir).flatMap((f) => [f, ...(statSync(join(dir, f)).isDirectory() ? names(join(dir, f)).map((n) => `${f}/${n}`) : [])]) : [];
  const found = names(ws);
  for (const old of ["meetings.jsonl", "listens.jsonl", "reflections.jsonl", "snapshots", "archive.json"]) {
    assert.ok(!found.includes(old), `${old} is not in the data root`);
  }
});

test("the minutes markdown shows the first failures only after a repair", () => {
  const m = minutes();
  assert.match(renderMinutesMarkdown(m), /## Repaired failures[\s\S]*does not resolve/);
  m.check.repaired = false;
  assert.doesNotMatch(renderMinutesMarkdown(m), /Repaired failures/);
});
