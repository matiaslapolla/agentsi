import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { ClaudeCall, ClaudeResult } from "./claude.ts";
import type { Product } from "./registry.ts";
import { loadStory, listStories } from "./story.ts";
import { proposeStory, reviseStory, suggestStories, type StoryProposeContext, type StoryContent } from "./story-propose.ts";
import { decideStory } from "./story-write.ts";
import { testDb } from "./test-db.ts";

const product: Product = { id: "p", name: "P", paths: ["/repo/p"], status: "active", roles: ["pm", "dev", "qa"] };
const usage = { model: "m", input_tokens: 1, output_tokens: 1, cache_read: 0, cache_write: 0, cost_usd: 0.05, duration_ms: 1 };

const content = (over: Partial<StoryContent> = {}): StoryContent => ({
  type: "story",
  title: "Ver el resultado",
  as_a: "entrenador",
  i_want: "ver el resultado",
  so_that: "decida rápido",
  criteria: [1, 2].map((n) => ({ given: `g${n}`, when: `w${n}`, then: `t${n}`, test: `tests/c${n}.test.ts`, checks: `Prueba ${n}.` })),
  owner_role: "dev",
  objective: null,
  sources: [],
  ...over,
});

/** A temp AgentSI root that is a git repo with a pm persona. */
async function withRoot(fn: (root: string) => Promise<void>): Promise<void> {
  const db = await testDb();
  const root = mkdtempSync(join(tmpdir(), "agentsi-propose-"));
  mkdirSync(join(root, "plugin", "agents"), { recursive: true });
  writeFileSync(join(root, "plugin", "agents", "pm.md"), "---\nname: pm\nmodel: sonnet\n---\n\nYou are pm.\n");
  const git = (...a: string[]) => spawnSync("git", ["-C", root, ...a], { encoding: "utf8" });
  git("init", "-q");
  git("-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init");
  return fn(root).finally(async () => {
    rmSync(root, { recursive: true, force: true });
    await db.close();
  });
}

const fake = (outputs: unknown[]) => {
  const calls: ClaudeCall[] = [];
  const caller = async (c: ClaudeCall): Promise<ClaudeResult> => {
    calls.push(c);
    const output = outputs[calls.length - 1];
    if (output === undefined) throw new Error("unexpected call");
    return { output, usage };
  };
  return { calls, caller };
};

const ctxFor = (root: string, caller: StoryProposeContext["caller"]): StoryProposeContext => ({
  product,
  config: { repo: root, test: "node --test {files}", gate: [], budget_usd: 5 },
  root,
  caller,
  snapshot: async () => "# Snapshot\n",
});

const gitLog = (root: string) => spawnSync("git", ["-C", root, "log", "--format=%s"], { encoding: "utf8" }).stdout;

test("a valid PM output stores the S-id story and makes no commit", () =>
  withRoot(async (root) => {
    const { calls, caller } = fake([{ stories: [content()] }]);
    const res = await proposeStory(ctxFor(root, caller), { text: "quiero ver el resultado" });
    assert.equal(res.stories[0]!.id, "S001");
    assert.equal(res.stories[0]!.proposed_by, "owner");
    assert.equal(res.stories[0]!.approval, "pending");
    assert.equal(res.totalCostUsd, 0.05);
    assert.equal((await loadStory("p", "S001", root)).slug, "ver-el-resultado");
    assert.deepEqual(calls[0]!.tools, ["Read", "Grep", "Glob"]);
    assert.equal(calls[0]!.cwd, root);
    assert.match(calls[0]!.prompt, /quiero ver el resultado/);
    assert.doesNotMatch(gitLog(root), /story/);
  }));

test("an invalid output is repaired once with the failure list", () =>
  withRoot(async (root) => {
    const bad = content({ criteria: content().criteria.slice(0, 1) });
    const { calls, caller } = fake([{ stories: [bad] }, { stories: [content()] }]);
    const res = await proposeStory(ctxFor(root, caller), { text: "x" });
    assert.equal(calls.length, 2);
    assert.match(calls[1]!.prompt, /criteria must hold 2 to 5 items, found 1/);
    assert.equal(res.usage.length, 2);
    assert.equal((await listStories("p", root)).length, 1);
  }));

test("two invalid outputs write nothing", () =>
  withRoot(async (root) => {
    const bad = { stories: [content({ owner_role: "ghost" })] };
    const { caller } = fake([bad, bad]);
    await assert.rejects(proposeStory(ctxFor(root, caller), { text: "x" }), /invalid after one repair.*owner_role "ghost"/s);
    assert.deepEqual(await listStories("p", root), []);
    assert.doesNotMatch(gitLog(root), /story/);
  }));

test("--type task rejects a model story of the other type", () =>
  withRoot(async (root) => {
    const task = content({ type: "task", as_a: null });
    const { calls, caller } = fake([{ stories: [content()] }, { stories: [task] }]);
    const res = await proposeStory(ctxFor(root, caller), { text: "x", type: "task" });
    assert.match(calls[1]!.prompt, /type must be "task"/);
    assert.equal(res.stories[0]!.id, "T001");
  }));

test("suggest saves up to 3 stories proposed by the pm and rejects 4", () =>
  withRoot(async (root) => {
    const three = [1, 2, 3].map((n) => content({ title: `Historia ${n}` }));
    const { caller } = fake([{ stories: [...three, content({ title: "Cuarta" })] }, { stories: three }]);
    const res = await suggestStories(ctxFor(root, caller));
    assert.deepEqual(res.stories.map((s) => s.id), ["S001", "S002", "S003"]);
    assert.ok(res.stories.every((s) => s.proposed_by === "pm"));
  }));

test("changes keeps id and slug, sets approval back to pending", () =>
  withRoot(async (root) => {
    const first = fake([{ stories: [content()] }]);
    await proposeStory(ctxFor(root, first.caller), { text: "x" });
    await decideStory({ product, root }, "S001", "approve").catch(() => {});
    const second = fake([{ stories: [content({ title: "Otro título", i_want: "algo distinto" })] }]);
    const res = await reviseStory(ctxFor(root, second.caller), "S001", "  hacelo distinto ");
    const s = await loadStory("p", "S001", root);
    assert.equal(res.stories[0]!.id, "S001");
    assert.equal(s.slug, "ver-el-resultado");
    assert.equal(s.title, "Otro título");
    assert.equal(s.approval, "pending");
    assert.equal(s.changes_note, "hacelo distinto");
    assert.equal((await listStories("p", root)).length, 1);
    assert.match(second.calls[0]!.prompt, /Note: hacelo distinto/);
  }));

test("approve and discard make no model call and commit", () =>
  withRoot(async (root) => {
    const { caller } = fake([{ stories: [content()] }]);
    await proposeStory(ctxFor(root, caller), { text: "x" });
    assert.equal((await decideStory({ product, root }, "S001", "approve")).approval, "approved");
    await assert.rejects(decideStory({ product, root }, "S001", "approve"), /only a pending story/);
    assert.equal((await decideStory({ product, root }, "S001", "discard")).status, "discarded");
    assert.doesNotMatch(gitLog(root), /story/);
  }));

test("a proposal may cite an open story, and a story source with an unknown id is repaired", () =>
  withRoot(async (root) => {
    await proposeStory(ctxFor(root, fake([{ stories: [content()] }]).caller), { text: "x" });
    const ghost = content({ title: "Otra", sources: [{ type: "story", ref: "S099" }] });
    const cited = content({ title: "Otra", sources: [{ type: "story", ref: "S001" }] });
    const { calls, caller } = fake([{ stories: [ghost] }, { stories: [cited] }]);
    const res = await proposeStory(ctxFor(root, caller), { text: "y" });
    assert.match(calls[1]!.prompt, /The story source "S099" is not a story of the product/);
    assert.deepEqual(res.stories[0]!.sources, [{ type: "story", ref: "S001" }]);
    assert.ok(JSON.stringify(calls[0]!.schema).includes('"enum":["commit","issue","path","url","story"]'));
  }));

test("the story prompt names the owner of the registry and asks for the owner's language", async () => {
  await withRoot(async (root) => {
    writeFileSync(join(root, "agentsi.config.json"), JSON.stringify({ owner: { name: "Ana", language: "Brazilian Portuguese" }, products: [product] }));
    const { calls, caller } = fake([{ stories: [content()] }]);
    await proposeStory(ctxFor(root, caller), { text: "quero ver o resultado" });
    const prompt = calls[0]!.prompt;
    assert.match(prompt, /A story request from Ana\./);
    assert.match(prompt, /Everything the user reads is in Brazilian Portuguese/);
    assert.match(prompt, /Do not propose what Ana already rejected/);
    assert.doesNotMatch(prompt, /Rioplatense|Spanish/i);
  });
});
