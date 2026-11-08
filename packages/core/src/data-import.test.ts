import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { Story } from "./story.ts";
import type { Minutes, UsageRow } from "./types.ts";

// The data root is read from AGENTSI_HOME when paths.ts loads, so the test home is set before the product code loads.
const base = mkdtempSync(join(tmpdir(), "agentsi-t028-import-"));
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

const { importData } = await import("./data-import.ts");
const { getPool } = await import("./db.ts");
const { testDb } = await import("./test-db.ts");
const { writeMinutes } = await import("./minutes.ts");
const { buildLogEntry } = await import("./meeting-log.ts");
const { updateReflections } = await import("./reflections.ts");
const { archiveMinutes } = await import("./archive.ts");

const story = (id: string, slug: string, title: string): Story => ({
  id,
  type: "story",
  slug,
  title,
  as_a: "usuario",
  i_want: "ver el resultado",
  so_that: "sepa qué pasó",
  criteria: [1, 2].map((n) => ({ given: `g${n}`, when: `w${n}`, then: `t${n}`, test: `tests/c${n}.test.ts`, checks: `c${n}` })),
  owner_role: "dev",
  objective: null,
  sources: [],
  status: "not_implemented",
  approval: "approved",
  changes_note: null,
  proposed_by: "owner",
  minutes: null,
  session: null,
  results: [],
  created_at: "2026-10-21T00:00:00.000Z",
  updated_at: "2026-10-21T00:00:00.000Z",
});

type Row = Record<string, unknown>;
const rowsOf = async (table: string): Promise<Row[]> => (await getPool().query(`select * from ${table}`)).rows as Row[];
const about = (rows: Row[], product: string, id: string) => rows.filter((r) => Object.values(r).includes(product) && Object.values(r).includes(id));
const holds = (row: Row, value: unknown) =>
  Object.values(row).some((v) => {
    try {
      assert.deepEqual(typeof v === "string" ? JSON.parse(v) : v, value);
      return true;
    } catch {
      return false;
    }
  });

test("agentsi data import loads memory, overlays and briefs, and a second run reports 0 changes", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const old = mkdtempSync(join(tmpdir(), "agentsi-old-root-"));
  t.after(() => rmSync(old, { recursive: true, force: true }));
  const ws = join(old, "workspace", "p");
  mkdirSync(join(ws, "memory"), { recursive: true });
  mkdirSync(join(ws, "agents"), { recursive: true });
  const items = [
    { id: "m-aaaa", text: "Uses Postgres 17", kind: "fact", helpful: 2, harmful: 0, source: "min-1", created_at: "2026-10-01T00:00:00.000Z", retired_at: null, retired_why: null },
    { id: "m-bbbb", text: "Never push on Fridays", kind: "lesson", helpful: 0, harmful: 1, source: "min-2", created_at: "2026-10-02T00:00:00.000Z", retired_at: "2026-10-03T00:00:00.000Z", retired_why: "stale" },
  ];
  const overlay = "---\nextends: dev\neffort: high\n---\nOnly the API.\n";
  const brief = {
    version: 1,
    product: { what: "A demo", audience: "Everyone", stage: "building" },
    objectives: [{ id: "o1", text: "Ship", signal: "released", due: "2027-01-01" }],
    north_stars: [],
    sources_of_truth: [],
    out_of_scope: [],
    roles: { dev: "judges cost", "dev.backend": "owns the API" },
    updated_at: "2026-10-13",
  };
  writeFileSync(join(ws, "memory", "dev.json"), JSON.stringify({ agent: "dev", product: "p", items }, null, 2) + "\n");
  writeFileSync(join(ws, "agents", "dev.backend.md"), overlay);
  writeFileSync(join(ws, "brief.json"), JSON.stringify(brief, null, 2) + "\n");
  for (const table of ["memory", "agent_overlays", "briefs"]) assert.deepEqual(await rowsOf(table), [], `${table} starts empty`);

  const first = await importData(old);
  assert.ok(first.changes >= 3, "the first run reports the imported rows");
  assert.ok((await rowsOf("memory")).length >= 1);
  const overlays = await rowsOf("agent_overlays");
  assert.equal(overlays.length, 1);
  assert.ok(Object.values(overlays[0]!).includes(overlay), "the overlay text is stored as it was in the file");
  const briefs = await rowsOf("briefs");
  assert.equal(briefs.length, 1);
  assert.ok(holds(briefs[0]!, brief));
  const stored = JSON.stringify(await rowsOf("memory"));
  assert.ok(stored.includes("Uses Postgres 17") && stored.includes("Never push on Fridays"), "every memory item is stored");

  const counts = async () => [(await rowsOf("memory")).length, (await rowsOf("agent_overlays")).length, (await rowsOf("briefs")).length];
  const before = await counts();
  const second = await importData(old);
  assert.equal(second.changes, 0, "the second run changes nothing");
  assert.deepEqual(await counts(), before);
});

test("agentsi data import loads old stories and logs with an upsert, and a second run reports 0 changes", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const old = mkdtempSync(join(tmpdir(), "agentsi-old-root-"));
  t.after(() => rmSync(old, { recursive: true, force: true }));
  const s1 = story("S001", "ver-resultado", "Ver el resultado");
  const s2 = story("T002", "limpiar", "Limpiar");
  const log1 = "[2026-10-21T00:00:00.000Z] Session qa (sonnet) starts.\n[2026-10-21T00:01:00.000Z] Every check passes.\n";
  for (const [product, s] of [["p", s1], ["p", s2], ["q", s1]] as const) {
    const dir = join(old, "workspace", product, "stories");
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, `${s.id}-${s.slug}.json`), JSON.stringify(s, null, 2) + "\n");
    writeFileSync(join(dir, `${s.id}-${s.slug}.md`), `# ${s.id} ${s.title}\n`);
  }
  writeFileSync(join(old, "workspace/p/stories/S001.log"), log1);
  assert.deepEqual(await rowsOf("stories"), []);

  const first = await importData(old);
  assert.ok(first.changes > 0, "the first run loads the old data");
  const stories = await rowsOf("stories");
  assert.equal(stories.length, 3, "two stories of p and one of q");
  assert.ok(holds(about(stories, "p", "S001")[0]!, s1));
  assert.ok(holds(about(stories, "p", "T002")[0]!, s2));
  assert.equal(about(stories, "q", "S001").length, 1, "keyed by product and story id");
  const logs = await rowsOf("story_logs");
  assert.equal(logs.length, 1);
  assert.ok(Object.values(about(logs, "p", "S001")[0]!).includes(log1), "the log text is stored as it was in the file");

  const second = await importData(old);
  assert.equal(second.changes, 0, "the second run changes nothing");
  assert.equal((await rowsOf("stories")).length, 3);
  assert.equal((await rowsOf("story_logs")).length, 1);
});

test("agentsi data import never replaces a row the database already has, because the database is newer", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const old = mkdtempSync(join(tmpdir(), "agentsi-old-root-"));
  t.after(() => rmSync(old, { recursive: true, force: true }));
  const stale = story("S001", "ver-resultado", "Ver el resultado");
  const dir = join(old, "workspace", "p", "stories");
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${stale.id}-${stale.slug}.json`), JSON.stringify(stale, null, 2) + "\n");
  writeFileSync(join(dir, "S001.log"), "old log\n");
  const newer = { ...stale, status: "implemented" };
  await getPool().query("insert into stories (product, id, data) values ($1, $2, $3::jsonb)", ["p", "S001", JSON.stringify(newer)]);
  await getPool().query("insert into story_logs (product, story_id, body) values ($1, $2, $3)", ["p", "S001", "old log\nnewer line\n"]);

  assert.equal((await importData(old)).changes, 0);
  assert.ok(holds(about(await rowsOf("stories"), "p", "S001")[0]!, newer), "the newer story in the database stays");
  assert.ok(Object.values(about(await rowsOf("story_logs"), "p", "S001")[0]!).includes("old log\nnewer line\n"), "the newer log stays");
});

const ID_A = "2026-10-20-0900-first";
const ID_B = "2026-10-21-0900-second";

const meetingOf = (id: string, question: string): Minutes => ({
  id,
  product: "demo",
  scope: null,
  question,
  created_at: `${id.slice(0, 10)}T09:00:00.000Z`,
  roles: { pm: { model: "sonnet", answer: `answer to ${question}`, priorities: [], concerns: [], memory_notes: [] } },
  priorities: [{ title: `Priority of ${question}`, why: "w", sources: [], owner_role: "pm", mark: null, is_new: false }],
  disagreements: [],
  decisions: [],
  script: [],
  check: { passed: true, failures: [], repaired: false },
  status: "valid",
});

const usageOf = (cost_usd: number): UsageRow[] => [
  { role: "pm", model: "sonnet", input_tokens: 1, output_tokens: 1, cache_read: 0, cache_write: 0, cost_usd, duration_ms: 10 },
];

const agentRun = { prompt: "p", memory: "", model: "sonnet", error: null, duration_ms: 1, unresolved_sources: [] };

async function dumpText(): Promise<string> {
  const db = getPool();
  const { rows: tables } = await db.query<{ table_name: string }>(
    "select table_name from information_schema.tables where table_schema = 'public' and table_type = 'BASE TABLE' and table_name <> 'schema_migrations' order by 1",
  );
  const out: Record<string, unknown[]> = {};
  for (const { table_name } of tables) out[table_name] = (await db.query(`select * from "${table_name}"`)).rows;
  return JSON.stringify(out);
}

test("agentsi data import inserts only the missing minutes, logs, listens, reflections, snapshots and archive records, and a second run changes nothing", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const old = mkdtempSync(join(tmpdir(), "agentsi-old-root-"));
  t.after(() => rmSync(old, { recursive: true, force: true }));

  // The database already holds meeting A, its listen, a reflection and an archive record, with other content than the old files.
  const dbA = meetingOf(ID_A, "DB QUESTION A");
  const dbEntry = buildLogEntry({ minutes: dbA, usage: usageOf(0.5), agents: { pm: agentRun }, snapshot: "" });
  await writeMinutes(dbA, usageOf(0.5), { entry: dbEntry, snapshot: "DB SNAPSHOT A" });
  mkdirSync(join(home, "workspace", "demo", "audio", ID_A), { recursive: true });
  writeFileSync(join(home, "workspace", "demo", "audio", ID_A, "call.webm"), "audio");
  const web = await import("../../../apps/web/lib/inbox-data.ts");
  await web.recordListen("demo", ID_A);
  const reflectionOf = (id: string, detail: string) => ({
    id,
    product: "demo",
    created_at: "2026-10-22T10:00:00.000Z",
    trigger: "manual" as const,
    signals: [{ agent: "pm", kind: "error" as const, ref: ID_A, detail }],
    memory: [],
    proposal: null,
    status: "applied" as const,
    commit: null,
  });
  await updateReflections("demo", (entries) => [...entries, reflectionOf("r-1", "DB REFLECTION DETAIL")]);
  await archiveMinutes("demo", ID_A);
  const before = await dumpText();
  assert.ok(before.includes("DB QUESTION A") && before.includes("DB SNAPSHOT A") && before.includes("DB REFLECTION DETAIL"), "the database holds its own rows");

  // The old data folder holds A with other content, and B, which the database lacks.
  const ws = join(old, "workspace", "demo");
  mkdirSync(join(ws, "minutes"), { recursive: true });
  mkdirSync(join(ws, "snapshots"), { recursive: true });
  const fileA = meetingOf(ID_A, "FILE QUESTION A");
  const fileB = meetingOf(ID_B, "FILE QUESTION B");
  writeFileSync(join(ws, "minutes", `${ID_A}.json`), JSON.stringify(fileA, null, 2) + "\n");
  writeFileSync(join(ws, "minutes", `${ID_A}.md`), "# FILE QUESTION A\n");
  writeFileSync(join(ws, "minutes", `${ID_A}.usage.json`), JSON.stringify(usageOf(0.424242)));
  writeFileSync(join(ws, "minutes", `${ID_B}.json`), JSON.stringify(fileB, null, 2) + "\n");
  writeFileSync(join(ws, "minutes", `${ID_B}.md`), "# FILE QUESTION B\n");
  writeFileSync(join(ws, "minutes", `${ID_B}.usage.json`), JSON.stringify(usageOf(0.77)));
  const entries = [fileA, fileB].map((m) => buildLogEntry({ minutes: m, usage: usageOf(0.1), agents: { pm: agentRun }, snapshot: "" }));
  writeFileSync(join(ws, "meetings.jsonl"), entries.map((e) => JSON.stringify(e) + "\n").join(""));
  writeFileSync(join(ws, "snapshots", `${ID_A}.md`), "FILE SNAPSHOT A\n");
  writeFileSync(join(ws, "snapshots", `${ID_B}.md`), "FILE SNAPSHOT B\n");
  writeFileSync(
    join(ws, "listens.jsonl"),
    [{ minutes_id: ID_A, at: "2026-10-22T10:10:10.000Z" }, { minutes_id: ID_B, at: "2026-10-22T11:11:11.000Z" }].map((l) => JSON.stringify(l) + "\n").join(""),
  );
  writeFileSync(
    join(ws, "reflections.jsonl"),
    [reflectionOf("r-1", "FILE REFLECTION DETAIL"), reflectionOf("r-2", "FILE REFLECTION TWO")].map((r) => JSON.stringify(r) + "\n").join(""),
  );
  writeFileSync(join(ws, "archive.json"), JSON.stringify({ minutes: [ID_A, ID_B] }, null, 2) + "\n");

  const first = await importData(old);
  assert.ok(first.changes > 0, "the first run inserts the missing rows");
  const after = await dumpText();

  for (const added of ["FILE QUESTION B", "FILE SNAPSHOT B", "2026-10-22T11:11:11", "FILE REFLECTION TWO"]) {
    assert.ok(after.includes(added), `the missing row is inserted: ${added}`);
  }
  assert.ok(after.includes("0.77"), "the usage of the missing meeting is inserted");
  assert.ok(after.includes(ID_B), "the missing meeting is in the log and the archive");
  for (const kept of ["DB QUESTION A", "DB SNAPSHOT A", "DB REFLECTION DETAIL"]) {
    assert.ok(after.includes(kept), `the existing row keeps its content: ${kept}`);
  }
  for (const replaced of ["FILE QUESTION A", "FILE SNAPSHOT A", "2026-10-22T10:10:10", "FILE REFLECTION DETAIL", "0.424242"]) {
    assert.ok(!after.includes(replaced), `the existing row is not replaced: ${replaced}`);
  }

  const second = await importData(old);
  assert.equal(second.changes, 0, "the second run changes nothing");
  assert.equal(await dumpText(), after, "the database is the same after the second run");
});

test("two products with the same old minutes id both import, the second as <id>-<product>, with its snapshot", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const old = mkdtempSync(join(tmpdir(), "agentsi-old-root-"));
  t.after(() => rmSync(old, { recursive: true, force: true }));
  for (const product of ["alpha", "beta"]) {
    await getPool().query("insert into products (id, name, paths, status) values ($1, $1, '[]', 'active') on conflict do nothing", [product]);
    const ws = join(old, "workspace", product);
    mkdirSync(join(ws, "minutes"), { recursive: true });
    mkdirSync(join(ws, "snapshots"), { recursive: true });
    writeFileSync(join(ws, "minutes", `${ID_A}.json`), JSON.stringify({ ...meetingOf(ID_A, `question of ${product}`), product }, null, 2));
    writeFileSync(join(ws, "snapshots", `${ID_A}.md`), `snapshot of ${product}\n`);
  }

  await importData(old);
  const rows = (await getPool().query<{ id: string; product: string }>("select id, product from minutes order by id")).rows;
  assert.deepEqual(rows, [
    { id: ID_A, product: "alpha" },
    { id: `${ID_A}-beta`, product: "beta" },
  ]);
  const snap = (await getPool().query<{ body: string }>("select body from snapshots where product = 'beta' and minutes_id = $1", [`${ID_A}-beta`])).rows;
  assert.deepEqual(snap, [{ body: "snapshot of beta\n" }]);
  assert.equal((await importData(old)).changes, 0, "the second run finds the renamed row");
});

test("the import copies the registry and the meeting audio into the data folder, and never replaces a file there", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const old = mkdtempSync(join(tmpdir(), "agentsi-old-root-"));
  const target = mkdtempSync(join(tmpdir(), "agentsi-data-folder-"));
  t.after(() => {
    rmSync(old, { recursive: true, force: true });
    rmSync(target, { recursive: true, force: true });
  });
  writeFileSync(join(old, "agentsi.config.json"), '{"products":[]}');
  writeFileSync(join(old, "lexicon.toml"), "[en]\n");
  mkdirSync(join(old, "voice", "transcripts"), { recursive: true });
  writeFileSync(join(old, "voice", "transcripts", "s.md"), "# Voice session s\n");
  for (const id of [ID_A, ID_B]) {
    mkdirSync(join(old, "workspace", "demo", "audio", id), { recursive: true });
    writeFileSync(join(old, "workspace", "demo", "audio", id, "call.webm"), `old audio ${id}`);
  }
  mkdirSync(join(target, "workspace", "demo", "audio", ID_B), { recursive: true });
  writeFileSync(join(target, "workspace", "demo", "audio", ID_B, "call.webm"), "newer audio");

  assert.equal((await importData(old, target)).files, 4, "the registry, the lexicon, the voice folder and the audio of A");
  assert.equal(readFileSync(join(target, "voice", "transcripts", "s.md"), "utf8"), "# Voice session s\n");
  assert.equal(readFileSync(join(target, "agentsi.config.json"), "utf8"), '{"products":[]}');
  assert.equal(readFileSync(join(target, "workspace", "demo", "audio", ID_A, "call.webm"), "utf8"), `old audio ${ID_A}`);
  assert.equal(readFileSync(join(target, "workspace", "demo", "audio", ID_B, "call.webm"), "utf8"), "newer audio");
  assert.equal((await importData(old, target)).changes, 0);
  assert.equal((await importData(target, target)).files, 0, "a data folder imported into itself copies nothing");
});
