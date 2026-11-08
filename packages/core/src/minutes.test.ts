import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { DecisionStore } from "./reflection-decision.ts";
import type { Minutes, UsageRow } from "./types.ts";

// The data root is read from AGENTSI_HOME when paths.ts loads, so the test home is set before the product code loads.
const base = mkdtempSync(join(tmpdir(), "agentsi-t028-minutes-"));
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

const { actionLine, addProductDecision, decide, minutesFileOf, newMinutesId, renderMinutesMarkdown, reserveMinutesId, writeMinutes } = await import("./minutes.ts");
const { minutesDir } = await import("./paths.ts");
const { buildLogEntry } = await import("./meeting-log.ts");
const { getPool } = await import("./db.ts");
const { testDb } = await import("./test-db.ts");

/** A minutes table of id -> product: the first insert of an id wins, and a later one returns the holder. */
function minutesTable(rows: Record<string, string> = {}): DecisionStore & { rows: Record<string, string> } {
  return {
    rows,
    async query<R extends object>(_sql: string, params: unknown[]) {
      const [id, product] = params as [string, string];
      rows[id] ??= product;
      return { rows: [{ product: rows[id] }] as unknown as R[] };
    },
  };
}

test("reserveMinutesId skips an id that a minutes file or another product's row holds", async () => {
  const root = mkdtempSync(join(tmpdir(), "agentsi-mid-"));
  try {
    const id = newMinutesId("The owner set a standing rule", new Date(2026, 9, 25, 2, 46));
    const meta = (product: string) => ({ id, product, scope: null, question: "q", status: "valid" as const, created_at: "2026-10-25T02:46:00Z" });
    const db = minutesTable();
    assert.equal(await reserveMinutesId(meta("lumen"), { root, db }), id);
    assert.equal(await reserveMinutesId(meta("orchard"), { root, db }), `${id}-2`, "the row of lumen holds the id");
    assert.deepEqual(db.rows, { [id]: "lumen", [`${id}-2`]: "orchard" });

    mkdirSync(minutesDir("harbor", root), { recursive: true });
    writeFileSync(join(minutesDir("harbor", root), `${id}-3.json`), "{}");
    assert.deepEqual(minutesFileOf(`${id}-3`, root), { file: join(minutesDir("harbor", root), `${id}-3.json`), product: "harbor" });
    assert.equal(await reserveMinutesId(meta("cedar"), { root, db }), `${id}-4`, "a file holds -3 with no row");
    assert.equal(minutesFileOf(`${id}-9`, root), null);
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("addProductDecision inserts a pending product decision with no minutes and returns its id", async () => {
  const seen: { sql: string; params: unknown[] }[] = [];
  const db: DecisionStore = {
    async query<R extends object>(sql: string, params: unknown[]) {
      seen.push({ sql, params });
      return { rows: [{ id: "41" }] as unknown as R[] };
    },
  };
  assert.equal(await addProductDecision("demo", "Escribir docs/product.md", "No hay north star.", undefined, db), 41);
  assert.match(seen[0]!.sql, /values \(\$1, 'product', null, \$2, \$3, \$4, 'pending'\)/);
  assert.deepEqual(seen[0]!.params, ["demo", "Escribir docs/product.md", "No hay north star.", null]);
  await addProductDecision("demo", "t", "w", "Run it", db);
  assert.match(seen[1]!.sql, /text, why, action, status\) values/);
  assert.deepEqual(seen[1]!.params, ["demo", "t", "w", "Run it"]);
});

function productDecisionDb(action: string | null): DecisionStore {
  return {
    async query<R extends object>(sql: string) {
      if (sql.startsWith("select")) return { rows: [{ product: "demo", kind: "product", reflection_id: null }] as unknown as R[] };
      return { rows: [{ product: "demo", minutes_id: null, action, pos: "0" }] as unknown as R[] };
    },
  };
}

test("decide returns the action of an approved decision and actionLine prints it", async () => {
  const res = await decide(5, "approved", { db: productDecisionDb("Escribir docs/product.md") });
  assert.equal(res.action, "Escribir docs/product.md");
  assert.equal(actionLine(res), "Action: Escribir docs/product.md");
});

test("actionLine prints nothing for a rejected decision or one without action", async () => {
  const rejected = await decide(5, "rejected", { db: productDecisionDb("Escribir docs/product.md") });
  assert.equal(actionLine(rejected), null);
  const plain = await decide(6, "approved", { db: productDecisionDb(null) });
  assert.equal(actionLine(plain), null);
});

test("minutes markdown shows the action after the reason, and nothing without it", () => {
  const base = { id: "m1", product: "demo", scope: null, question: "q", created_at: "2026-10-14T00:00:00Z", roles: {}, priorities: [], disagreements: [], script: [], check: { passed: true, failures: [], repaired: false }, status: "valid" as const };
  const withAction = renderMinutesMarkdown({ ...base, decisions: [{ text: "Cortar X", why: "Cuesta", action: "Borrar X" }] });
  assert.match(withAction, /- \[pending\] Cortar X — Cuesta → Borrar X/);
  const without = renderMinutesMarkdown({ ...base, decisions: [{ text: "Cortar X", why: "Cuesta" }] });
  assert.match(without, /- \[pending\] Cortar X — Cuesta\n/);
});

const usageRow = (role: string, cost_usd: number): UsageRow => ({
  role,
  model: "sonnet",
  input_tokens: 1000,
  output_tokens: 200,
  cache_read: 50,
  cache_write: 10,
  cost_usd,
  duration_ms: 1500,
});

const meeting = (): Minutes => ({
  id: "2026-10-30-0900-what-next",
  product: "demo",
  scope: null,
  question: "What do we build next?",
  created_at: "2026-10-30T09:00:00.000Z",
  roles: {
    pm: { model: "sonnet", answer: "Ship the import.", priorities: [], concerns: [], memory_notes: [] },
    dev: { model: "sonnet", answer: "Migrate the logs.", priorities: [], concerns: [], memory_notes: [] },
  },
  priorities: [{ title: "Ship the import", why: "The owner asked for it", sources: [], owner_role: "pm", mark: null, is_new: false }],
  disagreements: [],
  decisions: [{ text: "Drop commit.ts", why: "No data commits", action: "Delete it", status: "pending" }],
  script: [],
  check: { passed: true, failures: [], repaired: false },
  status: "valid",
});

/** Every row of every table, so the test does not depend on the table names of the migration. */
async function dumpDb(): Promise<Record<string, Record<string, unknown>[]>> {
  const db = getPool();
  const { rows: tables } = await db.query<{ table_name: string }>(
    "select table_name from information_schema.tables where table_schema = 'public' and table_type = 'BASE TABLE' and table_name <> 'schema_migrations' order by 1",
  );
  const out: Record<string, Record<string, unknown>[]> = {};
  for (const { table_name } of tables) out[table_name] = (await db.query(`select * from "${table_name}"`)).rows as Record<string, unknown>[];
  return out;
}

const parsed = (v: unknown): unknown => {
  if (typeof v !== "string") return v;
  try {
    return JSON.parse(v);
  } catch {
    return v;
  }
};

/** True when some column of some row holds `value`, as JSON or as the same text. */
function dbHolds(dump: Record<string, Record<string, unknown>[]>, value: unknown): boolean {
  return Object.values(dump).some((rows) =>
    rows.some((r) =>
      Object.values(r).some((v) => {
        try {
          assert.deepEqual(parsed(v), value);
          return true;
        } catch {
          return false;
        }
      }),
    ),
  );
}

function filesUnder(dir: string): string[] {
  if (!existsSync(dir)) return [];
  return readdirSync(dir).flatMap((f) => (statSync(join(dir, f)).isDirectory() ? filesUnder(join(dir, f)) : [join(dir, f)]));
}

test("minutes and per-role usage are saved in Postgres, read back as JSON and rendered Markdown, and no minutes file appears", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const m = meeting();
  const usage = [usageRow("pm", 0.11), usageRow("dev", 0.22), usageRow("synthesis", 0.33)];
  const entry = buildLogEntry({
    minutes: m,
    usage,
    agents: { pm: { prompt: "p", memory: "", model: "sonnet", error: null, duration_ms: 1, unresolved_sources: [] } },
    snapshot: "snapshot text",
  });
  await writeMinutes(m, usage, { entry, snapshot: "snapshot text" });

  // The database holds the minutes JSON and the usage of every role.
  const dump = await dumpDb();
  assert.ok(dbHolds(dump, m), "a row holds the minutes JSON");
  assert.ok(dbHolds(dump, usage), "a row holds the usage of all the roles");

  // The web workspace reads the minutes, the usage and the Markdown from the database.
  const web = await import("../../../apps/web/lib/workspace.ts");
  const read = await web.getMinutes("demo", m.id);
  assert.ok(read, "the web workspace finds the minutes");
  assert.equal(read.question, m.question);
  assert.deepEqual(read.priorities.map((p) => p.title), ["Ship the import"]);
  assert.deepEqual(read.decisions.map((d) => d.text), ["Drop commit.ts"]);
  assert.deepEqual(await web.getUsage("demo", m.id), usage);
  assert.match(renderMinutesMarkdown(read), /What do we build next\?/);

  // Nothing of the minutes reaches the disk.
  assert.deepEqual(filesUnder(minutesDir("demo")), [], "no file under workspace/demo/minutes");
  assert.deepEqual(filesUnder(join(home, "workspace", "demo")).filter((f) => f.includes(m.id)), [], "no file of the meeting under workspace/demo");
});
