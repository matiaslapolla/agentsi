import assert from "node:assert/strict";
import { mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { test } from "node:test";
import { archiveStory, loadStory, readStoryLog, restoreStory, saveStory, type Product, type Story } from "@agentsi/core";
import { testDb } from "@agentsi/core/test-db";
import { testHome } from "./test-home.ts";
import { storySubcommands, type StoryContext } from "./commands/story.ts";

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

/** The stories are in the database, so these tests call the story subcommands in process, on the in-process test database. */
async function withDb(t: { after: (fn: () => void | Promise<void>) => void }): Promise<void> {
  const db = await testDb();
  t.after(() => db.close());
}

test("story run refuses a story with approval pending and starts nothing", async (t) => {
  await withDb(t);
  const root = mkdtempSync(join(tmpdir(), "agentsi-story-run-cli-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const ctx: StoryContext = { product, config: { repo: "/repo/p", test: "t {files}", gate: [], budget_usd: 5 }, root };
  await saveStory(product, story, root);
  await assert.rejects(storySubcommands.run!(ctx, ["S001", "--product", "p"]), /Story S001 has approval pending; approve it before the run\./);
  assert.equal((await loadStory("p", "S001", root)).session, null);
  assert.equal(await readStoryLog("p", "S001"), "", "no log starts for a story that did not run");
  await assert.rejects(storySubcommands.review!(ctx, ["S001"]), /Story S001 has no story session\./);
  await assert.rejects(storySubcommands.accept!(ctx, ["S001"]), /no story session in state ready/);
  await assert.rejects(storySubcommands.reject!(ctx, ["S001"]), /Story S001 has no story session\./);
});

test("archiving a story with a running session is refused and the story stays active", async (t) => {
  await withDb(t);
  const h = testHome([{ id: "p", name: "P", roles: ["pm", "dev", "qa"], stories: { test: "node --test {files}", gate: ["true"] } }]);
  t.after(h.cleanup);
  const session = {
    state: "work" as const,
    branch: "story/S001-ver-resultado",
    worktree: join(h.repo, ".worktrees", "S001-ver-resultado"),
    base: "abc",
    tests_commit: "def",
    final_commit: null,
    attempts: 1,
    failure: null,
    log: "story_logs p/S001",
    started_at: "2026-10-21T00:00:00.000Z",
    pid: null,
  };
  await saveStory(product, { ...story, approval: "approved", session }, h.home);
  const before = await loadStory("p", "S001", h.home);
  await assert.rejects(archiveStory("p", "S001", h.home), /session.*running|running.*session/i);
  assert.deepEqual(await loadStory("p", "S001", h.home), before, "the story does not change");
});

test("story run refuses an archived story until restore brings it back", async (t) => {
  await withDb(t);
  const h = testHome([{ id: "p", name: "P", roles: ["pm", "dev", "qa"], stories: { test: "node --test {files}", gate: ["true"] } }]);
  t.after(h.cleanup);
  const ctx: StoryContext = { product, config: { repo: h.repo, test: "t {files}", gate: [], budget_usd: 5 }, root: h.home };
  await saveStory(product, { ...story, approval: "approved" }, h.home);
  assert.equal((await archiveStory("p", "S001", h.home)).error ?? null, null);

  await assert.rejects(storySubcommands.run!(ctx, ["S001", "--product", "p"]), /archived.*restore|restore.*archived/is);
  assert.equal((await loadStory("p", "S001", h.home)).session, null, "no session starts for an archived story");

  assert.equal((await restoreStory("p", "S001", h.home)).error ?? null, null);
  // Past the archive check the run stops at the worktree check (the product is no git repo), before any session starts.
  await assert.rejects(storySubcommands.run!(ctx, ["S001", "--product", "p"]), (err: Error) => !/archived|restore/i.test(err.message));
});

test("story review shows a budget stop with its cap", async (t) => {
  await withDb(t);
  const root = mkdtempSync(join(tmpdir(), "agentsi-story-run-cli-"));
  t.after(() => rmSync(root, { recursive: true, force: true }));
  const ctx: StoryContext = { product, config: { repo: join(root, "no-repo"), test: "t {files}", gate: [], budget_usd: 2 }, root };
  const failure = "Budget: the dev session reached its cap of USD 2.00 (stories.budget_usd in the registry) and Claude Code stopped it.";
  const session = {
    state: "failed" as const,
    branch: "story/S001-ver-resultado",
    worktree: "/repo/p/.worktrees/S001-ver-resultado",
    base: "abc",
    tests_commit: "def",
    final_commit: null,
    attempts: 1,
    failure,
    log: "story_logs p/S001",
    started_at: "2026-10-21T00:00:00.000Z",
    pid: null,
    stop: { reason: "budget" as const, cap_usd: 2 },
  };
  await saveStory(product, { ...story, approval: "approved", session }, root);
  const out: string[] = [];
  t.mock.method(console, "log", (line: string) => out.push(line));
  assert.equal(await storySubcommands.review!(ctx, ["S001"]), 0);
  const text = out.join("\n");
  assert.match(text, /Stopped: budget, cap USD 2\.00 for each session\./);
  assert.ok(text.includes(failure));
  assert.doesNotMatch(text, /Checks: failed/);
});

test("story result records a merge on the main branch and refuses a commit off it", async (t) => {
  await withDb(t);
  const h = testHome([{ id: "p", name: "P", roles: ["pm", "dev", "qa"], stories: { test: "node --test {files}", gate: ["true"] } }]);
  t.after(h.cleanup);
  const g = (...args: string[]) => spawnSync("git", args, { cwd: h.repo, encoding: "utf8", env: h.env }).stdout.trim();
  g("init", "-q", "-b", "main");
  g("commit", "-q", "--allow-empty", "-m", "fix by hand");
  const fix = g("rev-parse", "HEAD");
  g("checkout", "-q", "-b", "side");
  g("commit", "-q", "--allow-empty", "-m", "side");
  const side = g("rev-parse", "HEAD");
  const ctx: StoryContext = { product, config: { repo: h.repo, test: "t {files}", gate: [], budget_usd: 5 }, root: h.home };
  const discarded = { result: "discarded" as const, at: "2026-10-22T00:00:00.000Z", commit: null };
  await saveStory(product, { ...story, approval: "approved", results: [discarded] }, h.home);
  const before = await loadStory("p", "S001", h.home);

  await assert.rejects(storySubcommands.result!(ctx, ["S001", "merged_changed", "--commit", side, "--product", "p"]), /is not on the main branch main/);
  assert.deepEqual(await loadStory("p", "S001", h.home), before);
  await assert.rejects(storySubcommands.result!(ctx, ["S001", "discarded", "--commit", fix, "--product", "p"]), /agentsi story result <id>/);
  await assert.rejects(storySubcommands.result!(ctx, ["S001", "merged_changed", "--product", "p"]), /agentsi story result <id> merged_changed\|merged_unchanged --commit <sha>/);

  const out: string[] = [];
  t.mock.method(console, "log", (line: string) => out.push(line));
  assert.equal(await storySubcommands.result!(ctx, ["S001", "merged_changed", "--commit", fix.slice(0, 7), "--product", "p"]), 0);
  const after = await loadStory("p", "S001", h.home);
  assert.equal(after.status, "implemented");
  assert.deepEqual(after.results.map((r) => [r.result, r.commit]), [["discarded", null], ["merged_changed", fix]]);
  assert.match(out.join("\n"), /Story S001 is implemented, with the result merged_changed/);
});
