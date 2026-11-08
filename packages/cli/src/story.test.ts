import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";
import { saveStory, type Product, type Story } from "@agentsi/core";
import { testDb } from "@agentsi/core/test-db";
import { testHome } from "./test-home.ts";
import { storySubcommands, type StoryContext } from "./commands/story.ts";

const bin = fileURLToPath(new URL("../bin/agentsi.mjs", import.meta.url));
const h = testHome([
  { id: "agentsi", name: "AgentSI", roles: ["pm", "dev"], stories: { test: "node --test {files}", gate: ["true"] } },
  { id: "beta", name: "Beta", roles: ["pm"] },
]);
after(h.cleanup);
const run = (...args: string[]) => spawnSync(process.execPath, [bin, ...args], { encoding: "utf8", env: h.env, input: "" });

const product: Product = { id: "p", name: "P", paths: ["/repo/p"], status: "active", roles: ["pm", "dev", "qa"] };

const story: Story = {
  id: "S001",
  type: "story",
  slug: "ver-resultado",
  title: "Ver el resultado",
  as_a: "entrenador",
  i_want: "ver el resultado",
  so_that: "decida rápido",
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

async function capture(fn: () => Promise<unknown>): Promise<string> {
  const lines: string[] = [];
  const log = console.log;
  console.log = (...args: unknown[]) => void lines.push(args.join(" "));
  try {
    await fn();
  } finally {
    console.log = log;
  }
  return lines.join("\n");
}

test("story list and show read the stories of the product", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const root = mkdtempSync(join(tmpdir(), "agentsi-story-cli-"));
  try {
    const ctx: StoryContext = { product, config: { repo: "/repo/p", test: "t {files}", gate: [], budget_usd: 5 }, root };
    assert.equal(await capture(() => storySubcommands.list!(ctx, [])), "p has no stories.");
    await saveStory(product, story, root);
    assert.match(await capture(() => storySubcommands.list!(ctx, ["--product", "p"])), /^S001 {2}not_implemented {2}pending {2}- {2}dev {2}Ver el resultado$/);
    const md = await capture(() => storySubcommands.show!(ctx, ["S001"]));
    assert.match(md, /^# S001 Ver el resultado/);
    assert.match(md, /- \*\*Given\*\* g1/);
    assert.equal(JSON.parse(await capture(() => storySubcommands.show!(ctx, ["S001", "--json"]))).id, "S001");
  } finally {
    rmSync(root, { recursive: true, force: true });
  }
});

test("agentsi story refuses a product without a stories block and an unknown subcommand", () => {
  const off = run("story", "list", "--product", "beta");
  assert.equal(off.status, 1);
  assert.match(off.stderr, /Stories mode is off for beta: add a stories block to agentsi\.config\.json/);
  const bad = run("story", "bogus", "--product", "agentsi");
  assert.equal(bad.status, 1);
  assert.match(bad.stderr, /Unknown story subcommand "bogus"/);
});
