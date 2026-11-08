import assert from "node:assert/strict";
import { existsSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { getPool } from "./db.ts";
import { storiesDir } from "./paths.ts";
import { testDb } from "./test-db.ts";
import { storyConfig, type Product } from "./registry.ts";
import {
  approveStory,
  discardStory,
  listStories,
  loadStory,
  nextStoryId,
  requestChanges,
  saveStory,
  storyActionError,
  storyActions,
  storyMarkdown,
  storySlug,
  validateStory,
  type Criterion,
  type Story,
} from "./story.ts";

const PRODUCT: Product = { id: "p", name: "P", paths: ["/repo/p"], status: "active", roles: ["pm", "dev", "qa"] };
const ROSTER = { roster: PRODUCT.roles };

const criterion = (n: number, test: string | null = `tests/c${n}.test.ts`): Criterion => ({
  given: `un usuario ${n}`,
  when: `hace la acción ${n}`,
  then: `ve el resultado ${n}`,
  test,
  checks: `La acción ${n} da el resultado.`,
});

function story(overrides: Partial<Story> = {}): Story {
  return {
    id: "S001",
    type: "story",
    slug: "ver-resultado",
    title: "Ver el resultado",
    as_a: "entrenador",
    i_want: "ver el resultado",
    so_that: "decida rápido",
    criteria: [criterion(1), criterion(2)],
    owner_role: "dev",
    objective: null,
    sources: [{ type: "path", ref: "README.md" }],
    status: "not_implemented",
    approval: "pending",
    changes_note: null,
    proposed_by: "owner",
    minutes: null,
    session: null,
    results: [],
    created_at: "2026-10-21T00:00:00.000Z",
    updated_at: "2026-10-21T00:00:00.000Z",
    ...overrides,
  };
}

const task = (overrides: Partial<Story> = {}): Story =>
  story({ id: "T002", type: "task", slug: "limpiar-deuda", title: "Limpiar deuda", as_a: null, criteria: [criterion(1, null), criterion(2, null)], ...overrides });

test("a valid story and a valid task have no failures", () => {
  assert.deepEqual(validateStory(story(), ROSTER), []);
  assert.deepEqual(validateStory(task(), ROSTER), []);
});

test("the check rejects 1 criterion, 6 criteria, a null test in a story and an unknown owner_role", () => {
  assert.deepEqual(validateStory(story({ criteria: [criterion(1)] }), ROSTER), ["criteria must hold 2 to 5 items, found 1."]);
  const six = [1, 2, 3, 4, 5, 6].map((n) => criterion(n));
  assert.deepEqual(validateStory(story({ criteria: six }), ROSTER), ["criteria must hold 2 to 5 items, found 6."]);
  assert.deepEqual(validateStory(story({ criteria: [criterion(1), criterion(2, null)] }), ROSTER), [
    'criteria[2].test is null; a story of type "story" needs a test path.',
  ]);
  assert.deepEqual(validateStory(story({ owner_role: "marketing" }), ROSTER), ['owner_role "marketing" is not in the roster (pm, dev, qa).']);
});

test("the check rejects a story without a user, a wrong id prefix and test paths outside the repo", () => {
  assert.deepEqual(validateStory(story({ as_a: null }), ROSTER), ['as_a is empty; a story of type "story" needs a user.']);
  assert.deepEqual(validateStory(story({ id: "T001" }), ROSTER), ['id "T001" must start with S for type "story".']);
  assert.deepEqual(validateStory(task({ id: "S002" }), ROSTER), ['id "S002" must start with T for type "task".']);
  assert.deepEqual(validateStory(story({ id: "S1" }), ROSTER), ['id "S1" is not S or T followed by 3 or more digits.']);
  for (const bad of ["/abs/x.test.ts", "../x.test.ts", "tests/../../x.test.ts", "./x.test.ts", "tests\\x.test.ts", ""]) {
    assert.deepEqual(validateStory(story({ criteria: [criterion(1), criterion(2, bad)] }), ROSTER), [
      `criteria[2].test "${bad}" is not a repo-relative path.`,
    ], bad);
  }
  assert.deepEqual(validateStory(story({ approval: "changes_requested" }), ROSTER), ["changes_note is empty; approval changes_requested needs a note."]);
});

async function withDb(t: { after: (fn: () => void | Promise<void>) => void }): Promise<string> {
  const db = await testDb();
  t.after(() => db.close());
  await getPool().query("insert into products (id, name, paths, status) values ($1, $2, $3, $4)", ["p", "P", "[]", "active"]);
  const root = mkdtempSync(join(tmpdir(), "agentsi-story-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return root;
}

const storyRows = async (): Promise<Record<string, unknown>[]> => (await getPool().query("select * from stories")).rows as Record<string, unknown>[];

test("numbering gives S001, then T002, then S003 in one product", async (t) => {
  const root = await withDb(t);
  assert.equal(await nextStoryId("p", "story", root), "S001");
  await saveStory(PRODUCT, story(), root);
  const id = await nextStoryId("p", "task", root);
  assert.equal(id, "T002");
  await saveStory(PRODUCT, task({ id }), root);
  assert.equal(await nextStoryId("p", "story", root), "S003");
  assert.equal(await nextStoryId("other", "task", root), "T001");
  assert.equal((await storyRows()).length, 2, "both stories are in the stories table");
  assert.deepEqual(readdirSync(root), []);
});

test("save and load round trip through the database, with no files written", async (t) => {
  const root = await withDb(t);
  const saved = await saveStory(PRODUCT, story(), root);
  assert.notEqual(saved.updated_at, story().updated_at);
  assert.deepEqual(await loadStory("p", "S001", root), saved);
  assert.equal(storyMarkdown(await loadStory("p", "S001", root)), storyMarkdown(saved));
  await saveStory(PRODUCT, task(), root);
  assert.deepEqual((await listStories("p", root)).map((s) => s.id), ["S001", "T002"]);
  assert.deepEqual(await listStories("empty", root), []);
  await assert.rejects(async () => loadStory("p", "S009", root), /Unknown story "S009" in p/);
  assert.equal(existsSync(storiesDir("p", root)), false);
  assert.deepEqual(readdirSync(root), []);
});

test("save refuses an invalid story and writes nothing", async (t) => {
  const root = await withDb(t);
  await assert.rejects(async () => saveStory(PRODUCT, story({ owner_role: "nobody" }), root), /Story S001 is invalid:\n- owner_role "nobody"/);
  assert.deepEqual(await storyRows(), []);
  assert.equal(existsSync(storiesDir("p", root)), false);
});

test("a new slug updates the same stored story and leaves no second row or file", async (t) => {
  const root = await withDb(t);
  await saveStory(PRODUCT, story(), root);
  await saveStory(PRODUCT, story({ title: "Otro título", slug: storySlug("Otro título") }), root);
  assert.equal((await storyRows()).length, 1);
  assert.equal((await loadStory("p", "S001", root)).slug, "otro-titulo");
  assert.deepEqual(readdirSync(root), []);
});

test("the .md holds the story sentence and each criterion as given, when and then", () => {
  const md = storyMarkdown(story());
  for (const part of ["# S001 Ver el resultado", "**As a** entrenador", "**I want** ver el resultado", "**So that** decida rápido"]) {
    assert.ok(md.includes(part), part);
  }
  for (const n of [1, 2]) {
    assert.ok(md.includes(`- **Given** un usuario ${n}`));
    assert.ok(md.includes(`- **When** hace la acción ${n}`));
    assert.ok(md.includes(`- **Then** ve el resultado ${n}`));
    assert.ok(md.includes(`- Test: \`tests/c${n}.test.ts\``));
  }
  assert.ok(!storyMarkdown(task()).includes("**As a**"));
});

test("storySlug makes kebab-case without accents and cuts at a word", () => {
  assert.equal(storySlug("Ver el Resultado, ¡ya!"), "ver-el-resultado-ya");
  assert.equal(storySlug("Añadir campeón"), "anadir-campeon");
  assert.equal(storySlug("¿?"), "story");
  const long = storySlug("una frase muy larga que pasa del límite de cuarenta y ocho caracteres");
  assert.ok(long.length <= 48 && !long.endsWith("-"), long);
});

test("transitions: approve, request changes and discard follow the guard", () => {
  const pending = story();
  assert.deepEqual(storyActions(pending), ["approve", "changes", "discard"]);
  assert.match(storyActionError(pending, "run")!, /approval pending; approve it before the run/);

  const approved = approveStory(pending);
  assert.equal(approved.approval, "approved");
  assert.deepEqual(storyActions(approved), ["changes", "discard", "run"]);
  assert.throws(() => approveStory(approved), /only a pending story can be approved/);

  const changed = requestChanges(approved, "  más corto ");
  assert.equal(changed.approval, "changes_requested");
  assert.equal(changed.changes_note, "más corto");
  assert.throws(() => requestChanges(pending, " "), /needs a note/);

  const discarded = discardStory(pending);
  assert.equal(discarded.status, "discarded");
  assert.deepEqual(storyActions(discarded), []);
});

test("a story session holds the story until it fails; accept needs ready, reject needs a session", () => {
  const session = (state: NonNullable<Story["session"]>["state"]): Story["session"] => ({
    state,
    branch: "story/S001-ver-resultado",
    worktree: "/repo/p/.worktrees/S001-ver-resultado",
    base: "abc",
    tests_commit: null,
    final_commit: null,
    attempts: 0,
    failure: null,
    log: "workspace/p/stories/S001.log",
    started_at: "2026-10-21T00:00:00.000Z",
  });
  const running = story({ approval: "approved", session: session("work") });
  assert.deepEqual(validateStory(running, ROSTER), []);
  assert.deepEqual(storyActions(running), ["reject"]);
  assert.match(storyActionError(story({ approval: "approved" }), "reject")!, /has no story session/);
  assert.deepEqual(validateStory({ ...running, session: { ...running.session!, pid: 0 } }, ROSTER), ["session.pid must be a process id or null."]);
  assert.throws(() => discardStory(running), /story session in state work/);
  assert.throws(() => discardStory(story({ approval: "approved", session: session("failed") })), /Run agentsi story reject S001 first/);
  assert.deepEqual(storyActions(story({ approval: "approved", session: session("ready") })), ["accept", "reject"]);
  assert.deepEqual(storyActions(story({ approval: "approved", session: session("failed") })), ["changes", "run", "reject"]);
});

test("storyConfig resolves the repo and refuses a product without a stories block", () => {
  assert.throws(() => storyConfig(PRODUCT), /Stories mode is off for p: add a stories block to agentsi\.config\.json/);
  const on = { ...PRODUCT, stories: { test: "node --test {files}", gate: ["pnpm lint"] } };
  assert.deepEqual(storyConfig(on), { repo: "/repo/p", test: "node --test {files}", gate: ["pnpm lint"], budget_usd: 5 });
  assert.equal(storyConfig({ ...on, stories: { ...on.stories, repo: "/other" } }).repo, "/other");
  assert.throws(() => storyConfig({ ...on, stories: { test: "node --test", gate: [] } }), /test command with \{files\}/);
});

test("storyConfig takes budget_usd from the stories block and refuses a value that is not a positive number", () => {
  const on = (budget_usd: unknown) => ({ ...PRODUCT, stories: { test: "node --test {files}", gate: ["pnpm lint"], budget_usd: budget_usd as number } });
  assert.equal(storyConfig(on(1.5)).budget_usd, 1.5);
  for (const bad of [0, -1, "5", Number.NaN, Number.POSITIVE_INFINITY, null]) {
    assert.throws(() => storyConfig(on(bad)), /needs budget_usd as a positive number of USD/, String(bad));
  }
});

test("the Markdown view of a stored story is rendered from its stored JSON, and no stories/*.md file exists", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  await getPool().query("insert into products (id, name, paths, status) values ($1, $2, $3, $4)", ["p", "P", "[]", "active"]);
  const root = mkdtempSync(join(tmpdir(), "agentsi-story-md-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));

  const saved = await saveStory(PRODUCT, story({ title: "Título guardado" }), root);
  const rows = ((await getPool().query("select * from stories")).rows as Record<string, unknown>[]).filter((r) => Object.values(r).includes("S001"));
  assert.equal(rows.length, 1, "the story is in the stories table");

  const md = storyMarkdown(await loadStory("p", "S001", root));
  assert.equal(md, storyMarkdown(saved));
  for (const part of ["# S001 Título guardado", "**As a** entrenador", "- **Given** un usuario 1", "- **Then** ve el resultado 2"]) assert.ok(md.includes(part), part);

  assert.equal(existsSync(storiesDir("p", root)), false, "no stories folder, so no stories/*.md file");
  assert.deepEqual(readdirSync(root), []);
});

test("validation rejects a proposed_by that is not owner or pm", () => {
  assert.deepEqual(validateStory(story({ proposed_by: "someone" as never }), ROSTER), ['proposed_by "someone" is not owner or pm.']);
});
