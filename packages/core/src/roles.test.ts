import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { check } from "./check.ts";
import { readMemoryItems } from "./memory.ts";
import { AGENT_ID, agentRole, loadRole, parseAgentFile, type RoleDirs } from "./roles.ts";
import type { Product } from "./registry.ts";
import { testDb } from "./test-db.ts";
import type { Minutes } from "./types.ts";

const tmp = mkdtempSync(join(tmpdir(), "agentsi-roles-"));
const plugin = join(tmp, "plugin");
mkdirSync(plugin);
const stored = new Map<string, string>();
const dirs: RoleDirs = { plugin, overlay: async (p, id) => stored.get(`${p}/${id}`) ?? null };

const GLOBAL_DEV = "---\nname: dev\nmodel: sonnet\neffort: medium\ntools: Read, Grep, Glob, mcp__x__y\n---\n\nYou are dev.\n";
writeFileSync(join(plugin, "dev.md"), GLOBAL_DEV);
writeFileSync(join(plugin, "scout.md"), "---\nname: scout\nmodel: haiku\ntools: Read, Grep, Glob, WebSearch\n---\n\nYou are scout.\n");

function overlay(product: string, agent: string, text: string) {
  stored.set(`${product}/${agent}`, text);
}

overlay("p", "dev.backend", "---\nextends: dev\nmodel: opus\ntools: Read, Bash\n---\n\nFocus on the API.\n");
overlay("p", "seo", "---\nmodel: haiku\neffort: low\ntools: Read, Grep, WebSearch\n---\n\nYou are seo.\n");

test("a product with no overlays gets the global role unchanged", async () => {
  const def = await loadRole("dev", "empty", dirs);
  assert.deepEqual(def, {
    id: "dev",
    role: "dev",
    source: "global",
    model: "sonnet",
    effort: "medium",
    tools: ["Read", "Grep", "Glob"],
    body: parseAgentFile(GLOBAL_DEV).body,
  });
  assert.deepEqual((await loadRole("scout", undefined, dirs)).tools, ["Read", "Grep", "Glob", "WebSearch"]);
});

test("an overlay with extends overrides frontmatter keys and appends its body", async () => {
  const warn = console.warn;
  const warnings: string[] = [];
  console.warn = (m: string) => void warnings.push(m);
  try {
    const def = await loadRole("dev.backend", "p", dirs);
    assert.equal(def.source, "overlay");
    assert.equal(def.role, "dev");
    assert.equal(def.model, "opus");
    assert.equal(def.effort, "medium");
    assert.deepEqual(def.tools, ["Read"]);
    assert.equal(def.body, "You are dev.\n\n## This product\n\nFocus on the API.");
    assert.equal(warnings.length, 1);
    assert.match(warnings[0]!, /Bash/);
  } finally {
    console.warn = warn;
  }
});

test("an overlay without extends is a complete custom role", async () => {
  const def = await loadRole("seo", "p", dirs);
  assert.equal(def.source, "custom");
  assert.equal(def.role, "seo");
  assert.equal(def.model, "haiku");
  assert.equal(def.effort, "low");
  assert.deepEqual(def.tools, ["Read", "Grep", "WebSearch"]);
  assert.equal(def.body, "You are seo.");
  assert.equal(await agentRole("seo", "p", dirs), "seo");
});

test("a specialist without an overlay is an error", async () => {
  await assert.rejects(loadRole("dev.mobile", "p", dirs), /no overlay/);
  await assert.rejects(loadRole("dev.mobile", undefined, dirs), /no overlay/);
});

test("invalid ids and the template never load", async () => {
  for (const id of ["_template", "../x", "Dev", "dev.a.b", "dev."]) await assert.rejects(loadRole(id, "p", dirs), /Invalid agent id|Unknown role/);
  await assert.rejects(loadRole("nope", "p", dirs), /Unknown role/);
  assert.equal(await agentRole("dev.backend", "p", dirs), "dev");
  assert.equal(await agentRole("dev.other", "p", dirs), "dev");
});

test("roles and memory accept exactly the same agent ids", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const good = ["dev", "dev.backend", "growth-lead", "dev.backend-api", "seo"];
  const bad = ["growth-", "dev--ops", "-dev", "dev.", "dev.a.b", "Dev", "dev1", "dev_ops", "../x"];
  for (const id of good) {
    assert.ok(AGENT_ID.test(id), id);
    assert.deepEqual(await readMemoryItems("p", id), [], id);
  }
  for (const id of bad) {
    assert.ok(!AGENT_ID.test(id), id);
    await assert.rejects(readMemoryItems("p", id), /agent id/, id);
  }
});

test("the shipped template parses as a role file", () => {
  const text = readFileSync(join(import.meta.dirname, "../../../plugin/templates/role.md"), "utf8");
  const { frontmatter, body } = parseAgentFile(text);
  assert.equal(frontmatter.tools, "Read, Grep, Glob");
  assert.match(body, /You weigh:/);
  assert.doesNotMatch(body, /## How you work/, "shared rules live in rolePrompt, not in each role");
});

const product: Product = { id: "fixture", name: "Fixture", paths: [], status: "active", roles: ["pm", "dev.backend", "dev.mobile", "qa"] };

function minutes(answered: string[]): Minutes {
  return {
    id: "x",
    product: "fixture",
    scope: null,
    question: "q",
    created_at: "2026-10-13T00:00:00Z",
    roles: Object.fromEntries(
      product.roles.map((id) => [id, { model: "m", answer: answered.includes(id) ? "a" : "", priorities: [], concerns: [], memory_notes: [] }]),
    ),
    priorities: [],
    disagreements: [],
    decisions: [],
    script: [{ role: "pm", text: "hola" }],
    check: { passed: false, failures: [], repaired: false },
    status: "invalid",
  };
}

const roleFailures = async (answered: string[]) =>
  (await check(minutes(answered), { product })).failures.filter((f) => f.startsWith("Role "));

test("check passes the every-role-answered rule when one of two dev specialists answered", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  assert.deepEqual(await roleFailures(["pm", "dev.backend", "qa"]), []);
  assert.deepEqual(await roleFailures(["pm", "dev.mobile", "qa"]), []);
});

test("check names the role and its agents when none answered", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  assert.deepEqual(await roleFailures(["pm", "qa"]), ['Role "dev" has no answer (agents: dev.backend, dev.mobile).']);
  assert.deepEqual(await roleFailures(["dev.backend", "qa"]), ['Role "pm" has no answer.']);
});
