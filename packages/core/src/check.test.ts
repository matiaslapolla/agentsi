import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { check } from "./check.ts";
import { renderMinutesMarkdown } from "./minutes-md.ts";
import { getPool } from "./db.ts";
import type { Product } from "./registry.ts";
import { testDb } from "./test-db.ts";
import type { Minutes } from "./types.ts";

const repo = mkdtempSync(join(tmpdir(), "agentsi-check-"));
const sh = (...a: string[]) => execFileSync("git", a, { cwd: repo, encoding: "utf8" }).trim();
sh("init", "-q");
sh("config", "user.email", "t@t");
sh("config", "user.name", "t");
mkdirSync(join(repo, "docs"));
writeFileSync(join(repo, "CLAUDE.md"), "# Fixture\n");
writeFileSync(join(repo, "docs/spec.md"), "# Spec\n");
sh("add", ".");
sh("commit", "-qm", "init");
const hash = sh("rev-parse", "HEAD");

const product: Product = { id: "fixture", name: "Fixture", paths: [repo], status: "active", roles: ["pm", "dev"] };

const base = (): Minutes => ({
  id: "x",
  product: "fixture",
  scope: null,
  question: "q",
  created_at: "2026-10-11T00:00:00Z",
  roles: {
    pm: { model: "m", answer: "a", priorities: [], concerns: [], memory_notes: [] },
    dev: { model: "m", answer: "a", priorities: [], concerns: [], memory_notes: [] },
  },
  priorities: [
    {
      title: "t",
      why: "w",
      sources: [
        { type: "commit", ref: hash.slice(0, 8) },
        { type: "path", ref: "CLAUDE.md" },
        { type: "url", ref: "https://example.com/x" },
      ],
      owner_role: "pm",
      mark: null,
      is_new: false,
    },
  ],
  disagreements: [],
  decisions: [],
  script: [{ role: "pm", text: "hola" }],
  check: { passed: true, failures: [], repaired: false },
  status: "valid",
});

test("a well-formed minutes passes", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const r = await check(base(), { product });
  assert.deepEqual(r, { passed: true, failures: [] });
});

test("absolute path under the product counts as north star", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const m = base();
  m.priorities[0]!.sources = [{ type: "path", ref: join(repo, "docs/spec.md") }];
  assert.equal((await check(m, { product })).passed, true);
});

test("reports unresolved sources, missing north star, silent role and empty script", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const m = base();
  m.priorities[0]!.sources = [
    { type: "commit", ref: "deadbeef" },
    { type: "path", ref: "src/nope.ts" },
    { type: "url", ref: "not a url" },
  ];
  m.roles.dev!.answer = " ";
  m.script = [];
  const r = await check(m, { product });
  assert.equal(r.passed, false);
  assert.equal(r.failures.length, 6);
});

test("a story source must name a story of the product, and the script must not say its id", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const root = mkdtempSync(join(tmpdir(), "agentsi-check-stories-"));
  await getPool().query("insert into stories (product, id, data) values ($1, $2, $3::jsonb)", ["fixture", "S014", JSON.stringify({ id: "S014", title: "Filtrar la lista" })]);

  const m = base();
  m.priorities[0]!.sources.push({ type: "story", ref: "S014" });
  assert.deepEqual(await check(m, { product, root }), { passed: true, failures: [] });

  m.priorities[0]!.sources.push({ type: "story", ref: "S099" });
  m.script = [{ role: "pm", text: "Arrancamos con la S014, que ya está aprobada." }];
  const r = await check(m, { product, root });
  assert.deepEqual(r.failures, [
    'Priority #0: story source "S099" does not resolve.',
    'Call script names story S014 by its id: say its title "Filtrar la lista".',
  ]);
});

// S007: a decision lists the indexes of the priorities it covers.
const threePriorities = (decisions: Minutes["decisions"]): Minutes => {
  const m = base();
  m.priorities = [0, 1, 2].map((i) => ({ ...base().priorities[0]!, title: `Prioridad ${i}` }));
  m.decisions = decisions;
  return m;
};
const linkedDecision = (text: string, priorities: number[]) =>
  ({ text, why: "cuesta", action: "Borrar X", priorities }) as Minutes["decisions"][number];

test("minutes whose decisions link existing priority indexes pass, and the decision shows them", async () => {
  const root = mkdtempSync(join(tmpdir(), "agentsi-check-links-"));
  const good = threePriorities([linkedDecision("Cortar X", [0, 1]), linkedDecision("Subir Y", [2])]);
  assert.deepEqual(await check(good, { product, brief: null, root }), { passed: true, failures: [] });
  const line = renderMinutesMarkdown(good).split("\n").find((l) => l.includes("Cortar X"));
  assert.ok(line, "the decision is in the minutes");
  assert.match(line, /\b0\b/);
  assert.match(line, /\b1\b/);
});

test("minutes whose decision links a priority index that does not exist are rejected, naming the index", async () => {
  const root = mkdtempSync(join(tmpdir(), "agentsi-check-links-"));
  const bad = threePriorities([linkedDecision("Cortar X", [0, 5])]);
  const r = await check(bad, { product, brief: null, root });
  assert.equal(r.passed, false);
  assert.ok(
    r.failures.some((f) => /\b5\b/.test(f)),
    `the failures should name index 5, got: ${r.failures.join(" | ")}`,
  );
});
