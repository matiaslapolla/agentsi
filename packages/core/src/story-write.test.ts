import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readdirSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { Product } from "./registry.ts";
import { loadStory, saveStory, type Story } from "./story.ts";
import { decideStory, storyTransaction } from "./story-write.ts";
import { getPool } from "./db.ts";
import { testDb } from "./test-db.ts";

const product: Product = { id: "p", name: "P", paths: ["/repo/p"], status: "active", roles: ["pm", "dev", "qa"] };

const story: Story = {
  id: "S001",
  type: "story",
  slug: "ver-el-resultado",
  title: "Ver el resultado",
  as_a: "usuario",
  i_want: "ver el resultado",
  so_that: "sepa qué pasó",
  criteria: [1, 2].map((n) => ({ given: `g${n}`, when: `w${n}`, then: `t${n}`, test: `tests/c${n}.test.ts`, checks: `c${n}` })),
  owner_role: "dev",
  objective: null,
  sources: [],
  status: "not_implemented",
  approval: "pending",
  changes_note: null,
  proposed_by: "owner",
  minutes: null,
  session: null,
  results: [],
  created_at: "2026-10-21T00:00:00.000Z",
  updated_at: "2026-10-21T00:00:00.000Z",
};

/** A temp data root (for the workspace lock) and a migrated test database that holds the story. */
async function setup(t: { after: (fn: () => Promise<void> | void) => void }): Promise<string> {
  const db = await testDb();
  t.after(() => db.close());
  await getPool().query("insert into products (id, name, paths, status) values ($1, $2, $3, $4)", [product.id, product.name, "[]", "active"]);
  const dir = mkdtempSync(join(tmpdir(), "agentsi-story-write-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  await saveStory(product, story, dir);
  return dir;
}

test("decideStory approves, records a change note and discards", async (t) => {
  const dir = await setup(t);
  assert.equal((await decideStory({ product, root: dir }, "S001", "changes", " más corto ")).changes_note, "más corto");
  assert.equal((await loadStory("p", "S001", dir)).changes_note, "más corto");
  assert.equal((await decideStory({ product, root: dir }, "S001", "discard")).status, "discarded");
  await assert.rejects(decideStory({ product, root: dir }, "S001", "approve"), /is discarded/);
  assert.equal((await loadStory("p", "S001", dir)).status, "discarded", "a refused change leaves the story as it was");
});

test("a save that fails validation writes nothing", async (t) => {
  const dir = await setup(t);
  const before = await loadStory("p", "S001", dir);
  await assert.rejects(saveStory(product, { ...before, owner_role: "nobody" }, dir), /is invalid/);
  assert.deepEqual(await loadStory("p", "S001", dir), before);
});

test("story transactions of one product run one at a time", async (t) => {
  const dir = mkdtempSync(join(tmpdir(), "agentsi-story-lock-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  const order: string[] = [];
  const section = (name: string) => async () => {
    order.push(`${name}:in`);
    await new Promise((r) => setTimeout(r, 30));
    order.push(`${name}:out`);
  };
  await Promise.all([storyTransaction({ product, root: dir }, section("a")), storyTransaction({ product, root: dir }, section("b"))]);
  assert.deepEqual(order, ["a:in", "a:out", "b:in", "b:out"]);
});

/** A row of a table is about (product, id) when both values are among its column values, whatever the column names. */
const rowsFor = async (table: string, productId: string, id: string) =>
  ((await getPool().query(`select * from ${table}`)).rows as Record<string, unknown>[]).filter((r) => Object.values(r).includes(productId) && Object.values(r).includes(id));
const holdsJson = (row: Record<string, unknown>, value: unknown) =>
  Object.values(row).some((v) => {
    try {
      assert.deepEqual(typeof v === "string" ? JSON.parse(v) : v, value);
      return true;
    } catch {
      return false;
    }
  });

test("a story written to an empty database reads back unchanged, by product and id, and the data root gets no commit", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const other: Product = { ...product, id: "q", name: "Q" };
  for (const p of [product, other]) await getPool().query("insert into products (id, name, paths, status) values ($1, $2, $3, $4)", [p.id, p.name, "[]", "active"]);
  const dir = mkdtempSync(join(tmpdir(), "agentsi-story-db-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  execFileSync("git", ["init", "-q"], { cwd: dir });
  execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"], { cwd: dir });
  const commits = () => execFileSync("git", ["rev-list", "--count", "HEAD"], { cwd: dir, encoding: "utf8" }).trim();
  assert.deepEqual(await rowsFor("stories", "p", "S001"), []);

  const saved = await saveStory(product, story, dir);
  await saveStory(other, { ...story, title: "Otro producto", slug: "otro-producto" }, dir);

  assert.deepEqual(await loadStory("p", "S001", dir), saved);
  assert.equal((await loadStory("q", "S001", dir)).title, "Otro producto");
  const rows = await rowsFor("stories", "p", "S001");
  assert.equal(rows.length, 1);
  assert.ok(holdsJson(rows[0]!, saved), "the stories row holds the full story JSON");

  const approved = await decideStory({ product, root: dir }, "S001", "approve");
  assert.equal(approved.approval, "approved");
  assert.equal((await loadStory("p", "S001", dir)).approval, "approved");
  assert.equal(commits(), "1", "a story write makes no commit in the data root");
  assert.equal(execFileSync("git", ["status", "--porcelain"], { cwd: dir, encoding: "utf8" }), "", "and leaves no file in it");
});
