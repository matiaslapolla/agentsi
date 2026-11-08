import assert from "node:assert/strict";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";
import * as briefStore from "./brief.ts";
import { getPool } from "./db.ts";
import { git } from "./exec.ts";
import { sha256 } from "./meeting-log.ts";
import { readMemoryItems, writeMemoryItems, makeItem } from "./memory.ts";
import { pluginAgentsDir } from "./paths.ts";
import { writeReflections, type ReflectionEntry } from "./reflections.ts";
import { settleReflection } from "./reflection-decision.ts";
import type { Product } from "./registry.ts";
import { applyReset, planReset } from "./reset.ts";
import { applyPlan, planApply, registerableProduct, storiesChanged } from "./roster-apply.ts";
import * as rosterStore from "./roster.ts";
import { describeRoster, validateRoster } from "./roster.ts";
import { testDb } from "./test-db.ts";

const CONFIG = (repo: string) => `{
  "products": [
    { "id": "demo", "name": "Demo", "paths": ["${repo}"], "status": "active", "roles": ["pm", "dev"] },
    { "id": "other", "name": "Other", "paths": ["/nowhere"], "status": "active", "roles": ["pm"] }
  ]
}
`;

const brief = (roster: string[]) => ({
  version: 1,
  product: { what: "A demo", audience: "Everyone", stage: "building" },
  objectives: [{ id: "o1", text: "Ship", signal: "released", due: "2027-01-01" }],
  north_stars: ["docs/product.md"],
  sources_of_truth: [],
  out_of_scope: [],
  roles: Object.fromEntries(roster.map((id) => [id, `why ${id}`])),
  updated_at: "2026-10-13",
});

async function fixture() {
  const base = mkdtempSync(join(tmpdir(), "agentsi-roster-"));
  const root = join(base, "repo");
  const productRoot = join(base, "product");
  mkdirSync(join(root, "plugin/agents"), { recursive: true });
  mkdirSync(join(root, "workspace/demo/memory"), { recursive: true });
  mkdirSync(join(productRoot, "docs"), { recursive: true });
  writeFileSync(join(productRoot, "docs/product.md"), "# demo\n");
  for (const f of ["pm.md", "dev.md", "qa.md"]) cpSync(join(pluginAgentsDir(), f), join(root, "plugin/agents", f));
  writeFileSync(join(root, "agentsi.config.json"), CONFIG(productRoot));
  writeFileSync(join(root, "workspace/demo/memory/pm.json"), "{}\n");
  await git(root, "init", "-q");
  await git(root, "config", "user.email", "t@example.com");
  await git(root, "config", "user.name", "T");
  await git(root, "add", ".");
  await git(root, "commit", "-qm", "init");
  const product: Product = { id: "demo", name: "Demo", paths: [productRoot], status: "active", roles: ["pm", "dev"] };
  const stage = (roster: string[], overlays: Record<string, string> = {}, briefJson: unknown = brief(roster)) => {
    const dir = mkdtempSync(join(base, "stage-"));
    writeFileSync(join(dir, "brief.json"), JSON.stringify(briefJson, null, 2) + "\n");
    writeFileSync(join(dir, "roster.json"), JSON.stringify(roster));
    mkdirSync(join(dir, "agents"));
    for (const [id, text] of Object.entries(overlays)) writeFileSync(join(dir, "agents", `${id}.md`), text);
    return dir;
  };
  return { base, root, product, stage, cleanup: () => rmSync(base, { recursive: true, force: true }) };
}

const BACKEND = "---\nextends: dev\neffort: high\n---\nOnly the API.\n";
const commits = async (root: string) => Number((await git(root, "rev-list", "--count", "HEAD")).stdout.trim());

// Overlays, briefs and memory live in Postgres (T027): every test that reads or applies them runs on the test database.
const useDb = async (t: TestContext) => {
  const db = await testDb();
  t.after(() => db.close());
  return db;
};

test("validateRoster lists every problem of a roster", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    assert.deepEqual(await validateRoster(f.product, { root: f.root }), []);
    const bad = await validateRoster({ ...f.product, roles: ["pm", "dev.backend", "pm", "Bad"] }, { root: f.root });
    assert.ok(bad.some((m) => /repeated/.test(m)));
    assert.ok(bad.some((m) => /dev\.backend.*no overlay/.test(m)));
    assert.ok(bad.some((m) => /"Bad"/.test(m)));
    const entries = await describeRoster({ ...f.product, roles: ["pm", "dev.backend"] }, { root: f.root });
    assert.equal(entries[0]!.source, "global");
    assert.match(entries[1]!.error ?? "", /no overlay/);
  } finally {
    f.cleanup();
  }
});

test("validateRoster flags stray overlays, unknown parents and a brief that disagrees with the roster", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    await writeOverlay("demo", "qa", "---\nextends: nope\n---\nx\n");
    await writeBrief("demo", brief(["pm"]));
    const failures = await validateRoster(f.product, { root: f.root });
    assert.ok(failures.some((m) => /qa\.md.*not in the roster/.test(m)));
    assert.ok(failures.some((m) => /extends "nope"/.test(m)));
    assert.ok(failures.some((m) => /Brief: roles has no entry for roster agent "dev"/.test(m)));
  } finally {
    f.cleanup();
  }
});

test("apply refuses an invalid stage and writes nothing", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const dir = f.stage(["pm", "dev.backend"], {}, brief(["pm", "dev.backend", "extra"]));
    const { plan, failures } = await planApply({ stageDir: dir, product: f.product, register: false, root: f.root });
    assert.equal(plan, null);
    assert.ok(failures.length >= 2);
    assert.ok(failures.some((m) => /dev\.backend/.test(m)));
    assert.equal(existsSync(join(f.root, "workspace/demo/brief.json")), false);
    assert.equal(await briefStore.readBrief("demo"), null, "nothing reached the database");
    assert.match(readFileSync(join(f.root, "agentsi.config.json"), "utf8"), /"roles": \["pm", "dev"\] \}/);
    assert.equal((await planApply({ stageDir: join(f.base, "missing"), product: f.product, register: false, root: f.root })).failures.length, 2);
  } finally {
    f.cleanup();
  }
});

test("apply on a valid stage updates the registry, stores brief and overlays in the database and makes one registry commit", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const before = await commits(f.root);
    const dir = f.stage(["pm", "dev.backend", "dev.mobile", "qa"], { "dev.backend": BACKEND, "dev.mobile": "---\nextends: dev\n---\nApp.\n" });
    const { plan, failures } = await planApply({ stageDir: dir, product: f.product, register: false, root: f.root });
    assert.deepEqual(failures, []);
    assert.deepEqual(plan!.rosterAfter, ["pm", "dev.backend", "dev.mobile", "qa"]);
    assert.deepEqual(plan!.changes.map((c) => [c.path, c.status]), [["agents/dev.backend.md", "new"], ["agents/dev.mobile.md", "new"], ["brief.json", "new"]]);
    assert.equal(await commits(f.root), before);

    const commit = await applyPlan(plan!, dir, f.root);
    assert.equal(commit.committed, true);
    assert.equal(await commits(f.root), before + 1);
    assert.equal((await rosterStore.loadAgent("dev.backend", "demo", { root: f.root })).effort, "high");
    assert.ok(await briefStore.readBrief("demo"));
    assert.equal(existsSync(join(f.root, "workspace/demo/brief.json")), false);
    assert.equal(existsSync(join(f.root, "workspace/demo/agents")), false);
    const config = readFileSync(join(f.root, "agentsi.config.json"), "utf8");
    assert.equal(config, CONFIG(f.product.paths[0]!).replace('"roles": ["pm", "dev"]', '"roles": ["pm", "dev.backend", "dev.mobile", "qa"]'));
    const files = (await git(f.root, "show", "--name-only", "--format=", "HEAD")).stdout.trim().split("\n").sort();
    assert.deepEqual(files, ["agentsi.config.json"]);
    assert.equal((await git(f.root, "status", "--porcelain")).stdout.trim(), "");

    // Second stage: one overlay changes, one disappears.
    const next = f.stage(["pm", "dev.backend"], { "dev.backend": BACKEND.replace("high", "low") });
    const second = await planApply({ stageDir: next, product: { ...f.product, roles: plan!.rosterAfter }, register: false, root: f.root });
    assert.deepEqual(second.plan!.changes.map((c) => [c.path, c.status]), [["agents/dev.backend.md", "changed"], ["agents/dev.mobile.md", "removed"], ["brief.json", "changed"]]);
    assert.match(second.plan!.changes[0]!.diff, /-effort: high\n\+effort: low/);
    await applyPlan(second.plan!, next, f.root);
    await assert.rejects(Promise.resolve().then(() => rosterStore.loadAgent("dev.mobile", "demo", { root: f.root })), /no overlay/);
    assert.equal(await commits(f.root), before + 2);
  } finally {
    f.cleanup();
  }
});

test("apply with --register adds the product to the registry", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const fresh = registerableProduct("fresh", "Fresh", f.product.paths[0]);
    const dir = f.stage(["pm"]);
    const { plan, failures } = await planApply({ stageDir: dir, product: fresh, register: true, root: f.root });
    assert.deepEqual(failures, []);
    await applyPlan(plan!, dir, f.root);
    const config = JSON.parse(readFileSync(join(f.root, "agentsi.config.json"), "utf8"));
    assert.deepEqual(config.products.at(-1), { id: "fresh", name: "Fresh", paths: f.product.paths, status: "active", roles: ["pm"] });
    assert.ok(await briefStore.readBrief("fresh"), "the brief is in the database");
    assert.equal(existsSync(join(f.root, "workspace/fresh/brief.json")), false);
    assert.throws(() => registerableProduct("Fresh", "x", "/tmp"), /Invalid product id/);
    assert.throws(() => registerableProduct("a", undefined, "/tmp"), /needs --name/);
    assert.throws(() => registerableProduct("a", "A", "relative"), /absolute/);
  } finally {
    f.cleanup();
  }
});

const STORIES = { gate: ["pnpm typecheck", "pnpm lint"], test: "node --test {files}" };

test("apply with stories.json sets Stories mode in the same commit, and null turns it off", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const before = await commits(f.root);
    const dir = f.stage(["pm", "dev"]);
    writeFileSync(join(dir, "stories.json"), JSON.stringify(STORIES));
    const { plan, failures } = await planApply({ stageDir: dir, product: f.product, register: false, root: f.root });
    assert.deepEqual(failures, []);
    assert.equal(storiesChanged(plan!), true);
    await applyPlan(plan!, dir, f.root);
    const line = readFileSync(join(f.root, "agentsi.config.json"), "utf8").split("\n")[2];
    assert.match(line!, /"roles": \["pm", "dev"\], "stories": \{ "test": "node --test \{files\}", "gate": \["pnpm typecheck", "pnpm lint"\] \} \},$/);
    assert.equal(await commits(f.root), before + 1);
    assert.deepEqual((await git(f.root, "show", "--name-only", "--format=", "HEAD")).stdout.trim().split("\n").sort(), ["agentsi.config.json"]);

    // No stories.json: the block stays.
    const withBlock = { ...f.product, stories: { test: STORIES.test, gate: STORIES.gate } };
    const keep = await planApply({ stageDir: f.stage(["pm", "dev", "qa"]), product: withBlock, register: false, root: f.root });
    assert.equal(storiesChanged(keep.plan!), false);
    assert.deepEqual(keep.plan!.product.stories, withBlock.stories);

    // null: the block goes.
    const off = f.stage(["pm", "dev"]);
    writeFileSync(join(off, "stories.json"), "null");
    const drop = await planApply({ stageDir: off, product: withBlock, register: false, root: f.root });
    assert.equal(storiesChanged(drop.plan!), true);
    await applyPlan(drop.plan!, off, f.root);
    assert.equal(readFileSync(join(f.root, "agentsi.config.json"), "utf8"), CONFIG(f.product.paths[0]!));
  } finally {
    f.cleanup();
  }
});

test("apply refuses an invalid stories.json and writes nothing", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const before = await commits(f.root);
    const cases: [unknown, RegExp][] = [
      [{ test: "node --test", gate: ["pnpm lint"] }, /\{files\}/],
      [{ test: "node --test {files}", gate: [] as string[] }, /gate/],
      [{ test: "node --test {files}", gate: ["  "] }, /gate/],
      [{ ...STORIES, repo: "relative/path" }, /absolute path/],
      [{ ...STORIES, repo: "/nowhere/at/all" }, /absolute path/],
      [{ ...STORIES, tests: "x" }, /unknown keys/],
      [{ ...STORIES, budget_usd: 0 }, /budget_usd as a positive number/],
      [["pnpm lint"], /object or null/],
    ];
    for (const [block, re] of cases) {
      const dir = f.stage(["pm", "dev"]);
      writeFileSync(join(dir, "stories.json"), JSON.stringify(block));
      const { plan, failures } = await planApply({ stageDir: dir, product: f.product, register: false, root: f.root });
      assert.equal(plan, null);
      assert.match(failures.join("\n"), re);
    }
    assert.equal(await commits(f.root), before);
  } finally {
    f.cleanup();
  }
});

test("apply accepts budget_usd in stories.json and writes it after gate", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const dir = f.stage(["pm", "dev"]);
    writeFileSync(join(dir, "stories.json"), JSON.stringify({ budget_usd: 2, ...STORIES }));
    const { plan, failures } = await planApply({ stageDir: dir, product: f.product, register: false, root: f.root });
    assert.deepEqual(failures, []);
    await applyPlan(plan!, dir, f.root);
    const config = JSON.parse(readFileSync(join(f.root, "agentsi.config.json"), "utf8"));
    assert.deepEqual(Object.entries(config.products[0].stories), [["test", STORIES.test], ["gate", STORIES.gate], ["budget_usd", 2]]);
  } finally {
    f.cleanup();
  }
});

test("apply with --register and stories.json registers the product with its block", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const fresh = registerableProduct("fresh", "Fresh", f.product.paths[0]);
    const dir = f.stage(["pm"]);
    writeFileSync(join(dir, "stories.json"), JSON.stringify({ ...STORIES, repo: f.product.paths[0] }));
    const { plan, failures } = await planApply({ stageDir: dir, product: fresh, register: true, root: f.root });
    assert.deepEqual(failures, []);
    await applyPlan(plan!, dir, f.root);
    const config = JSON.parse(readFileSync(join(f.root, "agentsi.config.json"), "utf8"));
    assert.deepEqual(config.products.at(-1).stories, { repo: f.product.paths[0], test: STORIES.test, gate: STORIES.gate });
  } finally {
    f.cleanup();
  }
});

test("a stories change keeps extra keys and key order of the product line, and a bad edit fails the plan before any write", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const config = join(f.root, "agentsi.config.json");
    writeFileSync(config, readFileSync(config, "utf8").replace('"status": "active", "roles": ["pm", "dev"] }', '"roles": ["pm", "dev"], "status": "active", "notes": "x" }'));
    await git(f.root, "commit", "-qam", "odd line");
    const dir = f.stage(["pm", "dev"]);
    writeFileSync(join(dir, "stories.json"), JSON.stringify(STORIES));
    const { plan, failures } = await planApply({ stageDir: dir, product: f.product, register: false, root: f.root });
    assert.deepEqual(failures, []);
    await applyPlan(plan!, dir, f.root);
    const demo = JSON.parse(readFileSync(config, "utf8")).products[0];
    assert.deepEqual(Object.keys(demo), ["id", "name", "paths", "roles", "status", "notes", "stories"]);
    assert.equal((await git(f.root, "status", "--porcelain")).stdout.trim(), "");
  } finally {
    f.cleanup();
  }
});

test("apply refuses a stories repo that is a file", async (t) => {
  await useDb(t);
  const f = await fixture();
  try {
    const dir = f.stage(["pm", "dev"]);
    writeFileSync(join(dir, "stories.json"), JSON.stringify({ ...STORIES, repo: join(f.product.paths[0]!, "docs/product.md") }));
    const { plan, failures } = await planApply({ stageDir: dir, product: f.product, register: false, root: f.root });
    assert.equal(plan, null);
    assert.match(failures.join("\n"), /directory/);
  } finally {
    f.cleanup();
  }
});

// --- T027: memory, overlays and briefs persist in Postgres, with no data root commits for them ---

const writeOverlay = (product: string, agent: string, text: string): Promise<void> => {
  const fn = (rosterStore as Record<string, unknown>).writeOverlay as ((p: string, a: string, t: string) => Promise<void>) | undefined;
  assert.equal(typeof fn, "function", "roster.ts exports writeOverlay(product, agent, text)");
  return fn!(product, agent, text);
};
const writeBrief = (product: string, brief: unknown): Promise<void> => {
  const fn = (briefStore as Record<string, unknown>).writeBrief as ((p: string, b: unknown) => Promise<void>) | undefined;
  assert.equal(typeof fn, "function", "brief.ts exports writeBrief(product, brief)");
  return fn!(product, brief);
};

const lastCommitFiles = async (root: string) => (await git(root, "show", "--name-only", "--format=", "HEAD")).stdout.trim().split("\n").filter(Boolean);
const workspaceFilesInLog = async (root: string, since: string) =>
  (await git(root, "log", `${since}..HEAD`, "--name-only", "--format=", "--", "workspace")).stdout.trim();
const head = async (root: string) => (await git(root, "rev-parse", "HEAD")).stdout.trim();

test("a roster apply stores the brief and the overlays in the database and commits only the registry", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const f = await fixture();
  t.after(f.cleanup);
  const start = await head(f.root);
  const dir = f.stage(["pm", "dev.backend", "dev.mobile", "qa"], { "dev.backend": BACKEND, "dev.mobile": "---\nextends: dev\n---\nApp.\n" });
  const { plan, failures } = await planApply({ stageDir: dir, product: f.product, register: false, root: f.root });
  assert.deepEqual(failures, []);
  await applyPlan(plan!, dir, f.root);

  const agent = await rosterStore.loadAgent("dev.backend", "demo", { root: f.root });
  assert.equal(agent.source, "overlay");
  assert.equal(agent.effort, "high");
  assert.equal(agent.body.trim().endsWith("Only the API."), true);
  const stored = await briefStore.readBrief("demo");
  assert.deepEqual(Object.keys(stored!.roles), ["pm", "dev.backend", "dev.mobile", "qa"]);
  assert.equal(Number(((await getPool().query("select count(*) as n from agent_overlays")).rows[0] as { n: unknown }).n), 2);
  assert.equal(Number(((await getPool().query("select count(*) as n from briefs")).rows[0] as { n: unknown }).n), 1);

  assert.equal(existsSync(join(f.root, "workspace/demo/brief.json")), false, "no brief file");
  assert.equal(existsSync(join(f.root, "workspace/demo/agents")), false, "no overlay files");
  assert.equal(await workspaceFilesInLog(f.root, start), "", "no commit touched the workspace for these kinds");
  assert.deepEqual((await lastCommitFiles(f.root)).filter((p) => /brief\.json|\/agents\/|\/memory\//.test(p)), []);

  // A second stage removes one overlay: it leaves the database, and still no workspace commit.
  const next = f.stage(["pm", "dev.backend"], { "dev.backend": BACKEND.replace("high", "low") });
  const second = await planApply({ stageDir: next, product: { ...f.product, roles: plan!.rosterAfter }, register: false, root: f.root });
  await applyPlan(second.plan!, next, f.root);
  assert.equal((await rosterStore.loadAgent("dev.backend", "demo", { root: f.root })).effort, "low");
  await assert.rejects(Promise.resolve().then(() => rosterStore.loadAgent("dev.mobile", "demo", { root: f.root })), /no overlay/);
  assert.equal(await workspaceFilesInLog(f.root, start), "");
});

test("a reset removes memory and the overlay from the database and commits nothing in the workspace", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const f = await fixture();
  t.after(f.cleanup);
  const product: Product = { ...f.product, roles: ["pm", "dev", "dev.backend"] };
  await writeOverlay("demo", "dev.backend", BACKEND);
  await writeBrief("demo", { ...brief(["pm", "dev", "dev.backend"]), north_stars: [] });
  await writeMemoryItems("demo", "dev.backend", [makeItem("spec", { source: "m1", created_at: "2026-10-01T00:00:00.000Z", taken: [] })]);
  const start = await head(f.root);

  const plan = await planReset({ product, agent: "dev.backend", memory: true, root: f.root });
  assert.equal(plan.rosterAfter.includes("dev.backend"), false, "the specialist leaves the roster");
  await applyReset(plan, f.root);

  assert.deepEqual(await readMemoryItems("demo", "dev.backend"), []);
  await assert.rejects(Promise.resolve().then(() => rosterStore.loadAgent("dev.backend", "demo", { root: f.root })), /no overlay/);
  assert.equal(Object.keys((await briefStore.readBrief("demo"))!.roles).includes("dev.backend"), false, "its brief entry is gone");
  assert.equal(Number(((await getPool().query("select count(*) as n from agent_overlays")).rows[0] as { n: unknown }).n), 0);
  assert.equal(await workspaceFilesInLog(f.root, start), "", "no commit touched the workspace for these kinds");
});

test("an approved reflect proposal writes the overlay in the database and commits nothing in the workspace", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const f = await fixture();
  t.after(f.cleanup);
  const before = "---\nextends: dev\n---\nCite the plan.\n";
  const after = "---\nextends: dev\n---\nCite the plan. Cite only paths you read.\n";
  await writeOverlay("demo", "dev", before);
  const entry = {
    id: "r-1",
    product: "demo",
    created_at: "2026-10-13T15:30:00.000Z",
    trigger: "manual",
    signals: [],
    memory: [],
    proposal: {
      agent: "dev",
      target: "workspace/demo/agents/dev.md",
      trigger: { reason: "failure_class", failure_class: "unresolved_source", count: 3 },
      shared_lesson: null,
      rationale: "cites paths it never read",
      expected_effect: "fewer unresolved sources",
      diff: "",
      content: after,
      body_hash: "h",
      target_hash: sha256(before),
      replay: { tuning: [], held_out: [], rows: [], totals: null, verdict: { passed: true, worse: [], better: [] }, cost_rule: "", cost_usd: 0, budget_usd: 15, aborted: null },
      decision_id: 7,
      status: "pending",
    },
    status: "applied",
    commit: null,
  } as unknown as ReflectionEntry;
  writeReflections("demo", [entry], f.root);
  await git(f.root, "add", ".");
  await git(f.root, "commit", "-qm", "reflections");
  const start = await head(f.root);

  const res = await settleReflection("demo", "r-1", "approved", { root: f.root });
  assert.equal(res.status, "approved");
  // The store keeps the overlay as its file was, front matter included; the agent body ends with its text.
  assert.equal((await rosterStore.readOverlay("demo", "dev"))?.trim(), "---\nextends: dev\n---\nCite the plan. Cite only paths you read.");
  assert.ok((await rosterStore.loadAgent("dev", "demo", { root: f.root })).body.trim().endsWith("Cite the plan. Cite only paths you read."));
  assert.equal(existsSync(join(f.root, "workspace/demo/agents/dev.md")), false, "no overlay file");
  assert.equal((await git(f.root, "log", `${start}..HEAD`, "--name-only", "--format=", "--", "workspace/demo/agents", "workspace/demo/memory")).stdout.trim(), "");
});
