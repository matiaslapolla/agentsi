import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { test } from "node:test";
import { check } from "./check.ts";
import { briefHash, renderBriefXml, validateBrief, type Brief } from "./brief.ts";
import { rolePrompt, synthesisPrompt, synthesisSchemaFor } from "./meeting.ts";
import { renderMinutesMarkdown } from "./minutes-md.ts";
import { northStar } from "./snapshot.ts";
import { DEFAULT_OWNER, type Product } from "./registry.ts";
import type { Minutes } from "./types.ts";

const root = mkdtempSync(join(tmpdir(), "agentsi-brief-"));
mkdirSync(join(root, "docs"));
writeFileSync(join(root, "docs/product.md"), "# Product\n");
const product: Product = { id: "fixture", name: "Fixture", paths: [root], status: "active", roles: ["pm", "dev"] };

const brief = (over: Partial<Brief> = {}): Brief => ({
  version: 1,
  product: { what: "A <tool> & more", audience: "devs", stage: "building" },
  objectives: [
    { id: "o1", text: "Ship the beta", signal: "10 users", due: "2026-12-31" },
    { id: "o2", text: "Fix onboarding", signal: "activation > 40%", due: "2027-02-28" },
  ],
  north_stars: ["docs/product.md"],
  sources_of_truth: [{ kind: "issues", ref: "gh:acme/x", note: "backlog" }],
  out_of_scope: ["mobile app"],
  roles: { pm: "ranks", dev: "judges cost" },
  updated_at: "2026-10-13T00:00:00Z",
  ...over,
});

test("a complete brief validates", () => {
  assert.deepEqual(validateBrief(brief(), product, ["pm", "dev"]), []);
});

test("validateBrief lists every failure", () => {
  const bad = brief({
    product: { what: "", audience: "", stage: "dead" as never },
    objectives: [
      { id: "o1", text: "a", signal: "s", due: "2026-02-30" },
      { id: "o1", text: "b", signal: "s", due: "soon" },
    ],
    north_stars: ["docs/missing.md", "../outside.md", "/etc/passwd"],
    roles: { pm: "x", qa: "y" },
  });
  const failures = validateBrief(bad, product, ["pm", "dev"]);
  assert.equal(failures.filter((f) => f.includes("stage")).length, 1);
  assert.ok(failures.some((f) => f.includes('"o1" is repeated')));
  assert.equal(failures.filter((f) => f.includes("due date")).length, 2);
  assert.equal(failures.filter((f) => f.includes("north_stars")).length, 3);
  assert.ok(failures.some((f) => f.includes('"dev"')) && failures.some((f) => f.includes('"qa"')));
});

test("validateBrief bounds the objectives to 1..3", () => {
  const o = { id: "", text: "", signal: "", due: "2026-12-31" };
  assert.ok(validateBrief(brief({ objectives: [] }), product, ["pm", "dev"]).some((f) => f.includes("1 to 3")));
  const four = [1, 2, 3, 4].map((n) => ({ ...o, id: `o${n}` }));
  assert.ok(validateBrief(brief({ objectives: four }), product, ["pm", "dev"]).some((f) => f.includes("1 to 3")));
});

test("renderBriefXml escapes text and carries the objective ids", () => {
  const xml = renderBriefXml(brief());
  assert.ok(xml.startsWith("<brief>") && xml.endsWith("</brief>"));
  assert.ok(xml.includes('<objective id="o1" due="2026-12-31" signal="10 users">Ship the beta</objective>'));
  assert.ok(xml.includes("A &lt;tool&gt; &amp; more"));
  assert.ok(xml.includes("<path>docs/product.md</path>") && xml.includes("<item>mobile app</item>"));
});

test("briefHash changes with the content", () => {
  assert.equal(briefHash(brief()), briefHash(brief()));
  assert.notEqual(briefHash(brief()), briefHash(brief({ out_of_scope: [] })));
});

test("synthesis schema requires an objective id only with a brief", () => {
  const item = (b: Brief | null) => (synthesisSchemaFor(b).properties.priorities.items as { properties: Record<string, unknown>; required: string[] });
  assert.ok(!item(null).required.includes("objective") && !("objective" in item(null).properties));
  assert.ok(item(brief()).required.includes("objective"));
  assert.deepEqual((item(brief()).properties.objective as { enum: string[] }).enum, ["o1", "o2"]);
});

const promptOpts = { question: "q", snapshot: "SNAP", memory: "", cwd: "/", scope: null, role: "dev", web: false, meeting: true, owner: DEFAULT_OWNER };

test("role prompt places the brief before the snapshot and adds the brief rules", () => {
  const withBrief = rolePrompt({ ...promptOpts, brief: brief() });
  assert.ok(withBrief.indexOf("<brief>") < withBrief.indexOf("SNAP"));
  assert.ok(withBrief.includes("north stars declared in the <brief>") && withBrief.includes("<out_of_scope>"));
  const without = rolePrompt({ ...promptOpts, brief: null });
  assert.ok(!without.includes("<brief>") && !without.includes("declared in the <brief>"));
});

test("synthesis prompt lists the objective ids before the snapshot", () => {
  const withBrief = synthesisPrompt("q", {}, "COMPACT", brief(), DEFAULT_OWNER);
  assert.ok(withBrief.includes("o1, o2") && withBrief.indexOf("<brief>") < withBrief.indexOf("COMPACT"));
  assert.ok(!synthesisPrompt("q", {}, "COMPACT", null, DEFAULT_OWNER).includes("objective"));
});

test("minutes markdown shows the objective", () => {
  const m = {
    id: "x", product: "fixture", scope: null, question: "q", created_at: "t", roles: {}, disagreements: [], decisions: [], script: [],
    priorities: [{ title: "T", why: "w", sources: [], owner_role: "pm", objective: "o2", mark: null, is_new: false }],
    check: { passed: true, failures: [], repaired: false }, status: "valid",
  } satisfies Minutes;
  assert.ok(renderMinutesMarkdown(m).includes("**T** (pm) [o2]"));
});

const repo = mkdtempSync(join(tmpdir(), "agentsi-check-brief-"));
execFileSync("git", ["init", "-q"], { cwd: repo });
mkdirSync(join(repo, "docs"));
writeFileSync(join(repo, "CLAUDE.md"), "# Fixture\n");
writeFileSync(join(repo, "docs/product.md"), "# Product\n");
writeFileSync(join(repo, "docs/other.md"), "# Other\n");

const checkProduct: Product = { id: "fixture", name: "Fixture", paths: [repo], status: "active", roles: ["pm"] };


const minutesWith = (path: string, objective: string | null): Minutes => ({
  id: "x", product: "fixture", scope: null, question: "q", created_at: "2026-10-13T00:00:00Z",
  roles: { pm: { model: "m", answer: "a", priorities: [], concerns: [], memory_notes: [] } },
  priorities: [{ title: "t", why: "w", sources: [{ type: "path", ref: path }], owner_role: "pm", objective, mark: null, is_new: false }],
  disagreements: [], decisions: [], script: [{ role: "pm", text: "hola" }],
  check: { passed: true, failures: [], repaired: false }, status: "valid",
});

const run = (m: Minutes, b: ReturnType<typeof brief> | null) => check(m, { product: checkProduct, brief: b });

test("with a brief, an objective id and a declared north star pass", async () => {
  assert.deepEqual(await run(minutesWith("docs/product.md", "o1"), brief()), { passed: true, failures: [] });
});

test("with a brief, an unknown or missing objective fails", async () => {
  for (const objective of ["o9", null]) {
    const r = await run(minutesWith("docs/product.md", objective), brief());
    assert.equal(r.passed, false);
    assert.ok(r.failures.some((f) => f.includes("must be one of the brief ids: o1, o2")));
  }
});

test("with a brief, a north-star doc that is not declared no longer counts", async () => {
  for (const path of ["docs/other.md", "CLAUDE.md"]) {
    const r = await run(minutesWith(path, "o1"), brief());
    assert.equal(r.passed, false);
    assert.ok(r.failures.some((f) => f.includes("declared in the brief")));
  }
});

test("with a brief and no declared north stars, the north-star rule is skipped", async () => {
  const r = await run(minutesWith("docs/other.md", "o1"), brief({ north_stars: [] }));
  assert.deepEqual(r, { passed: true, failures: [] });
});

test("without a brief, today's rules apply and objective is ignored", async () => {
  assert.deepEqual(await run(minutesWith("docs/other.md", null), null), { passed: true, failures: [] });
});

const REPO = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const OLD_PATH_HELPERS = ["memory" + "Path", "memory" + "Dir", "agents" + "Dir", "brief" + "Path"];

function sourceFiles(dir: string): string[] {
  const out: string[] = [];
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (["node_modules", ".next", "dist", ".turbo", ".worktrees"].includes(entry.name)) continue;
    const full = join(dir, entry.name);
    if (entry.isDirectory()) out.push(...sourceFiles(full));
    else if (/\.(ts|tsx|mts|js|mjs)$/.test(entry.name) && !/\.test\.[a-z]+$/.test(entry.name)) out.push(full);
  }
  return out;
}

test("no code outside data-import.ts uses the old file path helpers", () => {
  const files = [...sourceFiles(join(REPO, "packages")), ...sourceFiles(join(REPO, "apps"))];
  assert.ok(files.length > 20, "the scan sees the source of packages and apps");
  const matches = new Set<string>();
  for (const file of files) {
    const text = readFileSync(file, "utf8");
    if (OLD_PATH_HELPERS.some((name) => new RegExp(`\\b${name}\\b`).test(text))) matches.add(relative(REPO, file));
  }
  assert.deepEqual([...matches], ["packages/core/src/data-import.ts"]);
});

test("snapshot lists the declared north stars before the default ones, once each", () => {
  writeFileSync(join(root, "CLAUDE.md"), "# Claude\n");
  const target = { path: root, prefix: "", scope: null };
  const text = northStar(product, [target], brief({ north_stars: ["docs/product.md", "CLAUDE.md"] }));
  assert.ok(text.indexOf("### docs/product.md (declared in the brief)") < text.indexOf("### CLAUDE.md (declared in the brief)"));
  assert.equal(text.match(/# Claude/g)?.length, 1);
  assert.ok(!northStar(product, [target], null).includes("declared in the brief"));
});
