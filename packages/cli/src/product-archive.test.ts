import assert from "node:assert/strict";
import { existsSync, mkdirSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { after, test } from "node:test";
import { testHome } from "./test-home.ts";

// S015: the owner archives a whole product with `agentsi archive`, and restores it with `agentsi restore`.
// The product keeps its registry entry and its data. The archive record is a row of the database, so the commands
// run in this process against the in-process test database; `meet` and `story run` are only run while the product is
// archived, where they refuse before they reach a model.

const ARCHIVED = "helix";
const ACTIVE = "orbit";
const STORY = "S001";
const ID = "2026-10-21-0900-roadmap";

const stories = { test: "node --test {files}", gate: ["true"] };
const h = testHome([
  { id: ARCHIVED, name: "Helix", roles: ["pm", "dev", "qa"], stories },
  { id: ACTIVE, name: "Orbit", roles: ["pm", "dev"] },
]);
after(h.cleanup);

// Core reads the data root when it loads, so AGENTSI_HOME points at the test home before core is imported.
process.env.AGENTSI_HOME = h.home;
const { getPool, loadRegistry, getProduct } = await import("@agentsi/core");
const { testDb } = await import("@agentsi/core/test-db");
const { archive, restore } = await import("./commands/archive.ts");
const { meet } = await import("./commands/meet.ts");
const { story } = await import("./commands/story.ts");
const { whoami } = await import("./commands/whoami.ts");
const { assertNotArchived } = await import("./context.ts");
const { runCommand } = await import("./run-command.ts");

/** The product has minutes in the database and a minutes folder of an old data root that was never imported. */
async function given(t: { after(fn: () => unknown): void }) {
  const db = await testDb();
  t.after(() => db.close());
  await getPool().query("insert into minutes (id, product, data) values ($1, $2, $3::jsonb)", [
    ID,
    ARCHIVED,
    JSON.stringify({ id: ID, product: ARCHIVED, question: "What do we build next?", status: "valid", created_at: "2026-10-21T09:00:00Z", roles: {}, priorities: [], decisions: [] }),
  ]);
  const minutes = join(h.home, "workspace", ARCHIVED, "minutes");
  mkdirSync(minutes, { recursive: true });
  writeFileSync(join(minutes, `${ID}.md`), "# What do we build next?\n");
}

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

/** The paths whose text differs between two snapshots, added and removed ones included. */
function differing(before: Map<string, string>, after: Map<string, string>): string[] {
  return [...new Set([...before.keys(), ...after.keys()])].filter((p) => before.get(p) !== after.get(p));
}

/** The line of `whoami --list` that starts with the product id. */
function lineOf(text: string, id: string): string {
  return text.split("\n").find((l) => l.trim().split(/\s+/)[0] === id) ?? "";
}

const productRows = async () => (await getPool().query("select kind, ref from archive_records where product = $1", [ARCHIVED])).rows;

test("agentsi archive succeeds, keeps the registry entry and the product data, and records the archive in the database", async (t) => {
  await given(t);
  const registryBefore = loadRegistry(join(h.home, "agentsi.config.json")).products.find((p) => p.id === ARCHIVED);
  const dataBefore = snapshot(h.home);
  assert.deepEqual(await productRows(), [], "the product has no archive record before");

  const r = await runCommand(archive, ["--product", ARCHIVED]);

  assert.equal(r.status, 0, r.stderr);
  const registryAfter = loadRegistry(join(h.home, "agentsi.config.json")).products.find((p) => p.id === ARCHIVED);
  assert.ok(registryAfter, "the registry entry of the product is still there");
  assert.equal(registryAfter.name, registryBefore?.name);
  assert.deepEqual(registryAfter.paths, registryBefore?.paths);
  assert.deepEqual(differing(dataBefore, snapshot(h.home)), [], "the data root is as it was: the archive writes no file");
  assert.equal(existsSync(join(h.home, "workspace", ARCHIVED, "archive.json")), false, "the archive record is a database row, not a file");
  assert.deepEqual(await productRows(), [{ kind: "product", ref: "" }], "the product is in the archive record");
  assert.equal((await getPool().query("select 1 from minutes where id = $1", [ID])).rows.length, 1, "the minutes of the product are still stored");
});

test("agentsi whoami --list still lists an archived product and marks it as archived", async (t) => {
  await given(t);
  assert.equal((await runCommand(archive, ["--product", ARCHIVED])).status, 0, "the product is archived first");

  const r = await runCommand(whoami, ["--list"]);

  assert.equal(r.status, 0, r.stderr);
  const archived = lineOf(r.stdout, ARCHIVED);
  assert.ok(archived, "the archived product is still in the list");
  assert.match(archived, /archiv/i, "its line marks it as archived");
  assert.doesNotMatch(lineOf(r.stdout, ACTIVE), /archiv/i, "the active product is not marked");
});

test("agentsi meet and agentsi story run refuse an archived product, exit with an error and say to run agentsi restore", async (t) => {
  await given(t);
  assert.equal((await runCommand(archive, ["--product", ARCHIVED])).status, 0, "the product is archived first");
  const dataBefore = snapshot(h.home);

  const refused = await runCommand(meet, ["--product", ARCHIVED, "--question", "What do we build next?"]);
  assert.notEqual(refused.status, 0, "meet exits with an error");
  assert.match(refused.stdout + refused.stderr, /agentsi restore/, "meet says to run agentsi restore");

  const storyRun = await runCommand(story, ["run", STORY, "--product", ARCHIVED]);
  assert.notEqual(storyRun.status, 0, "story run exits with an error");
  assert.match(storyRun.stdout + storyRun.stderr, /agentsi restore/, "story run says to run agentsi restore");

  assert.deepEqual(differing(dataBefore, snapshot(h.home)), [], "the refused commands change nothing in the data root");
});

test("agentsi restore clears the archived mark, and the product is no longer refused", async (t) => {
  await given(t);
  assert.equal((await runCommand(archive, ["--product", ARCHIVED])).status, 0, "the product is archived first");
  await assert.rejects(assertNotArchived(getProduct(ARCHIVED)), /agentsi restore/, "an archived product is refused");

  const r = await runCommand(restore, ["--product", ARCHIVED]);

  assert.equal(r.status, 0, r.stderr);
  assert.deepEqual(await productRows(), [], "the archive record is empty again");
  const listed = await runCommand(whoami, ["--list"]);
  assert.doesNotMatch(lineOf(listed.stdout, ARCHIVED), /archiv/i, "the archived mark is gone");
  await assert.doesNotReject(assertNotArchived(getProduct(ARCHIVED)), "the restored product is not refused");
});
