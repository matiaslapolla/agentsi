import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";
import { after, test } from "node:test";
import { testHome } from "./test-home.ts";

// Core reads the data root when it loads, so AGENTSI_HOME points at the test home before core is imported.
// The commands run in this process, against the in-process test database that `getPool()` points at.
const h = testHome([{ id: "agentsi", name: "AgentSI", roles: ["pm", "dev"] }]);
after(h.cleanup);
process.env.AGENTSI_HOME = h.home;
const { getPool } = await import("@agentsi/core");
const { testDb } = await import("@agentsi/core/test-db");
const { archive, restore } = await import("./commands/archive.ts");
const { runCommand } = await import("./run-command.ts");

const ID = "2026-10-21-0900-roadmap";
const MINUTES = {
  id: ID,
  product: "agentsi",
  scope: null,
  question: "What do we build next?",
  status: "valid",
  created_at: "2026-10-21T09:00:00Z",
  roles: {},
  priorities: [{ title: "Ship the roadmap", why: "the owner asked", owner_role: "pm", objective: null, sources: [], mark: null, is_new: false }],
  decisions: [],
};

/** A test database with one saved meeting of the product. */
async function given(t: { after(fn: () => unknown): void }) {
  const db = await testDb();
  t.after(() => db.close());
  await getPool().query("insert into minutes (id, product, data) values ($1, 'agentsi', $2::jsonb)", [ID, JSON.stringify(MINUTES)]);
}

const run = (command: typeof archive, ...args: string[]) => runCommand(command, [...args, "--product", "agentsi"]);

/** The text of every file under `dir`, keyed by its path relative to `dir`. Git metadata is left out. */
function snapshot(dir: string): Map<string, string> {
  const files = new Map<string, string>();
  const walk = (at: string) => {
    for (const entry of readdirSync(at, { withFileTypes: true })) {
      if (entry.name === ".git") continue;
      const path = join(at, entry.name);
      if (entry.isDirectory()) walk(path);
      else files.set(relative(dir, path), readFileSync(path, "latin1"));
    }
  };
  walk(dir);
  return files;
}

const archived = async () => (await getPool().query("select ref from archive_records where product = 'agentsi' and kind = 'minutes'")).rows.map((r: { ref: string }) => r.ref);

test("agentsi archive records the minutes in the database and leaves the minutes row, the data root and the repo unchanged", async (t) => {
  await given(t);
  const rowBefore = (await getPool().query("select data from minutes where id = $1", [ID])).rows;
  const dataBefore = snapshot(h.home);
  const repoBefore = snapshot(h.repo);

  const r = await run(archive, ID);

  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(await archived(), [ID], "the archive record names the minutes");
  assert.deepEqual((await getPool().query("select data from minutes where id = $1", [ID])).rows, rowBefore, "the minutes row keeps its content");
  assert.deepEqual(snapshot(h.home), dataBefore, "the data root gets no file");
  assert.deepEqual(snapshot(h.repo), repoBefore, "the product repo is not touched");
});

test("agentsi restore takes an archived minutes out of the archive record and leaves the data root as it was", async (t) => {
  await given(t);
  const dataBefore = snapshot(h.home);
  assert.equal((await run(archive, ID)).status, 0, "the minutes is archived first");

  const r = await run(restore, ID);

  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(await archived(), [], "the archive record is empty again");
  assert.deepEqual(snapshot(h.home), dataBefore, "the data root is as it was before the archive");
  assert.notEqual((await run(restore, ID)).status, 0, "restoring minutes that are not archived is refused");
});
