import assert from "node:assert/strict";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test, type TestContext } from "node:test";
import { readBrief, writeBrief, type Brief } from "./brief.ts";
import { git } from "./exec.ts";
import { makeItem, memoryAgents, writeMemoryItems } from "./memory.ts";
import { pluginAgentsDir } from "./paths.ts";
import type { Product } from "./registry.ts";
import { applyReset, globalRoles, isNoop, planReset } from "./reset.ts";
import { readOverlays, writeOverlay } from "./roster.ts";
import { testDb } from "./test-db.ts";

const CONFIG = `{
  "products": [
    { "id": "demo", "name": "Demo", "paths": ["/p"], "status": "active", "roles": ["pm", "dev.backend", "dev", "seo"] }
  ]
}
`;

async function fixture(t: TestContext) {
  const db = await testDb();
  t.after(() => db.close());
  const root = mkdtempSync(join(tmpdir(), "agentsi-reset-"));
  mkdirSync(join(root, "plugin/agents"), { recursive: true });
  for (const f of ["pm.md", "dev.md", "qa.md", "social.md", "scout.md"]) cpSync(join(pluginAgentsDir(), f), join(root, "plugin/agents", f));
  writeFileSync(join(root, "agentsi.config.json"), CONFIG);
  await writeOverlay("demo", "dev", "---\nextends: dev\n---\nx\n");
  await writeOverlay("demo", "dev.backend", "---\nextends: dev\n---\nx\n");
  await writeOverlay("demo", "seo", "You are SEO.\n");
  for (const a of ["pm", "dev", "dev.backend", "seo"]) {
    await writeMemoryItems("demo", a, [makeItem("note", { source: "m1", created_at: "2026-10-01T00:00:00.000Z", taken: [] })]);
  }
  const brief = { roles: { pm: "a", "dev.backend": "b", dev: "c", seo: "d" }, other: 1 } as unknown as Brief;
  await writeBrief("demo", brief);
  await git(root, "init", "-q");
  await git(root, "config", "user.email", "t@example.com");
  await git(root, "config", "user.name", "T");
  await git(root, "add", ".");
  await git(root, "commit", "-qm", "init");
  const product: Product = { id: "demo", name: "Demo", paths: ["/p"], status: "active", roles: ["pm", "dev.backend", "dev", "seo"] };
  t.after(() => rmSync(root, { recursive: true, force: true }));
  return { root, product };
}

const count = async (root: string) => Number((await git(root, "rev-list", "--count", "HEAD")).stdout.trim());

test("globalRoles lists the plugin roles in the registry order", async (t) => {
  const f = await fixture(t);
  assert.deepEqual(globalRoles(f.root), ["pm", "social", "dev", "qa", "scout"]);
});

test("resetting a global role removes only its overlay; --memory adds its memory", async (t) => {
  const f = await fixture(t);
  const plan = await planReset({ product: f.product, agent: "dev", memory: false, root: f.root });
  assert.deepEqual(plan.removals, ["agents/dev.md"]);
  assert.deepEqual(plan.rosterAfter, f.product.roles);
  assert.deepEqual((await planReset({ product: f.product, agent: "dev", memory: true, root: f.root })).removals, ["agents/dev.md", "memory/dev.json"]);
  const before = await count(f.root);
  const commit = await applyReset(plan, f.root);
  assert.equal(commit.committed, false, "nothing in the data root changed");
  assert.equal(await count(f.root), before);
  assert.deepEqual(Object.keys(await readOverlays("demo")).sort(), ["dev.backend", "seo"]);
  assert.ok((await memoryAgents("demo")).includes("dev"));
  assert.equal(readFileSync(join(f.root, "agentsi.config.json"), "utf8"), CONFIG);
});

test("resetting a specialist or a custom role removes it from the roster and the brief", async (t) => {
  const f = await fixture(t);
  const plan = await planReset({ product: f.product, agent: "dev.backend", memory: true, root: f.root });
  assert.deepEqual(plan.rosterAfter, ["pm", "dev", "seo"]);
  assert.deepEqual(plan.removals, ["agents/dev.backend.md", "memory/dev.backend.json"]);
  const before = await count(f.root);
  await applyReset(plan, f.root);
  assert.match(readFileSync(join(f.root, "agentsi.config.json"), "utf8"), /"roles": \["pm", "dev", "seo"\]/);
  assert.deepEqual(await readBrief("demo"), { roles: { pm: "a", dev: "c", seo: "d" }, other: 1 });
  assert.equal(await count(f.root), before + 1, "only the registry is committed");
  assert.deepEqual((await git(f.root, "show", "--name-only", "--format=", "HEAD")).stdout.trim().split("\n"), ["agentsi.config.json"]);
  assert.equal((await git(f.root, "status", "--porcelain")).stdout.trim(), "");
});

test("reset --all drops brief and overlays, keeps the global roles of the roster and keeps memory unless asked", async (t) => {
  const f = await fixture(t);
  const keep = await planReset({ product: f.product, agent: null, memory: false, root: f.root });
  assert.deepEqual(keep.removals, ["brief.json", "agents/dev.backend.md", "agents/dev.md", "agents/seo.md"]);
  assert.deepEqual(keep.rosterAfter, ["pm", "dev"]);
  const none = await planReset({ product: { ...f.product, roles: ["dev.backend", "seo"] }, agent: null, memory: false, root: f.root });
  assert.deepEqual(none.rosterAfter, ["pm", "social", "dev", "qa", "scout"]);
  const all = await planReset({ product: f.product, agent: null, memory: true, root: f.root });
  assert.equal(all.removals.filter((r) => r.startsWith("memory/")).length, 4);
  await applyReset(keep, f.root);
  assert.equal(await readBrief("demo"), null);
  assert.deepEqual(await readOverlays("demo"), {});
  assert.equal((await memoryAgents("demo")).length, 4);
  assert.equal(existsSync(join(f.root, "workspace")), false);
  assert.match(readFileSync(join(f.root, "agentsi.config.json"), "utf8"), /"roles": \["pm", "dev"\]/);
});

test("reset refuses an unknown agent and an empty roster, and reports a no-op", async (t) => {
  const f = await fixture(t);
  await assert.rejects(planReset({ product: f.product, agent: "ghost", memory: false, root: f.root }), /not in the roster/);
  await assert.rejects(planReset({ product: { ...f.product, roles: ["seo"] }, agent: "seo", memory: false, root: f.root }), /empty roster/);
  assert.equal(isNoop(await planReset({ product: f.product, agent: "pm", memory: false, root: f.root })), true);
});
