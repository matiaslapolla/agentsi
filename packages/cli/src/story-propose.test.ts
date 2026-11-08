import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { listStories, loadStory, saveStory, type ClaudeCall, type ClaudeCaller, type Product, type Story } from "@agentsi/core";
import { testDb } from "@agentsi/core/test-db";
import { proposeCommands } from "./commands/story-propose.ts";
import type { StoryContext } from "./commands/story.ts";

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

test("story approve and discard change the stored story, and usage errors name the command", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const root = mkdtempSync(join(tmpdir(), "agentsi-story-propose-cli-"));
  execFileSync("git", ["init", "-q"], { cwd: root });
  execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", "commit", "-q", "--allow-empty", "-m", "init"], { cwd: root });
  const log = console.log;
  const err = console.error;
  console.log = () => {};
  console.error = () => {};
  try {
    const ctx: StoryContext = { product, config: { repo: "/repo/p", test: "t {files}", gate: [], budget_usd: 5 }, root };
    await saveStory(product, story, root);
    await proposeCommands.approve(ctx, ["S001", "--product", "p"]);
    assert.equal((await loadStory("p", "S001", root)).approval, "approved");
    await proposeCommands.discard(ctx, ["S001"]);
    assert.equal((await loadStory("p", "S001", root)).status, "discarded");
    await assert.rejects(proposeCommands.approve(ctx, []), /agentsi story approve <id>/);
    await assert.rejects(proposeCommands.changes(ctx, ["S001"]), /agentsi story changes <id>/);
    await assert.rejects(proposeCommands.new(ctx, ["x", "--type", "bug"]), /--type must be/);
  } finally {
    console.log = log;
    console.error = err;
    rmSync(root, { recursive: true, force: true });
  }
});

// ---- S002: story proposals cite only sources that exist ----

const git = (cwd: string, ...args: string[]) =>
  execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", ...args], { cwd, encoding: "utf8" });

const usage = { model: "m", input_tokens: 1, output_tokens: 1, cache_read: 0, cache_write: 0, cost_usd: 0.05, duration_ms: 1 };

/** A data root (git repo with a pm persona and the story S001) and a product repo with one committed file. */
async function setup() {
  const db = await testDb();
  const base = mkdtempSync(join(tmpdir(), "agentsi-s002-"));
  const root = join(base, "data");
  const repo = join(base, "product");
  mkdirSync(join(root, "plugin", "agents"), { recursive: true });
  writeFileSync(join(root, "plugin", "agents", "pm.md"), "---\nname: pm\nmodel: sonnet\n---\n\nYou are pm.\n");
  git(root, "init", "-q");
  git(root, "commit", "-q", "--allow-empty", "-m", "init");
  mkdirSync(join(repo, "src"), { recursive: true });
  writeFileSync(join(repo, "src", "app.ts"), "export const app = 1;\n");
  git(repo, "init", "-q");
  git(repo, "add", ".");
  git(repo, "commit", "-q", "-m", "add app");
  const commit = git(repo, "rev-parse", "HEAD").trim();
  await saveStory(product, story, root);
  return {
    base,
    root,
    repo,
    commit,
    cleanup: async () => {
      rmSync(base, { recursive: true, force: true });
      await db.close();
    },
  };
}

type Fixture = Awaited<ReturnType<typeof setup>>;

/** A fake PM: each call returns the next output; a call with no output left throws and is still recorded. */
function fakePM(outputs: unknown[]) {
  const calls: ClaudeCall[] = [];
  const caller: ClaudeCaller = async (c) => {
    calls.push(c);
    const output = outputs[calls.length - 1];
    if (output === undefined) throw new Error(`unexpected PM call ${calls.length}`);
    return { output, usage };
  };
  return { calls, caller };
}

const criteria = [1, 2].map((n) => ({ given: `g${n}`, when: `w${n}`, then: `t${n}`, test: `tests/c${n}.test.ts`, checks: `Prueba ${n}.` }));

/** The PM's output for one story that cites the given sources. */
const proposal = (sources: { type: string; ref: string }[], title = "Ver el resultado") => ({
  stories: [
    { type: "story", title, as_a: "entrenador", i_want: "ver el resultado", so_that: "decida rápido", criteria, owner_role: "dev", objective: null, sources },
  ],
});

/** The CLI context, plus the two seams tests use in place of the model and the snapshot. */
const ctxFor = (s: Fixture, caller: ClaudeCaller): StoryContext & { caller: ClaudeCaller; snapshot: () => Promise<string> } => ({
  product,
  config: { repo: s.repo, test: "node --test {files}", gate: [], budget_usd: 5 },
  root: s.root,
  caller,
  snapshot: async () => "# Snapshot\n",
});

/** Runs a command the way the CLI does: prints go to a buffer, a thrown error is returned. */
async function run(fn: () => Promise<unknown>): Promise<{ out: string; error: Error | null }> {
  const lines: string[] = [];
  const log = console.log;
  const err = console.error;
  console.log = (...a: unknown[]) => lines.push(a.map(String).join(" "));
  console.error = (...a: unknown[]) => lines.push(a.map(String).join(" "));
  let error: Error | null = null;
  try {
    await fn();
  } catch (e) {
    error = e as Error;
  } finally {
    console.log = log;
    console.error = err;
  }
  return { out: lines.join("\n"), error };
}

/** The Failures list of the repair prompt that the PM got on its second call. */
const failuresOf = (prompt: string): string => prompt.split("Failures:")[1]?.split("Previous output:")[0] ?? "";

const UNKNOWN_COMMIT = "0123456789abcdef0123456789abcdef01234567";

test("S002.1: a proposal whose path, commit and story id all exist is saved with no source failure and no repair", async () => {
  const s = await setup();
  try {
    const pm = fakePM([proposal([
      { type: "path", ref: "src/app.ts" },
      { type: "commit", ref: s.commit },
      { type: "story", ref: "S001" },
    ])]);
    const { out, error } = await run(() => proposeCommands.new(ctxFor(s, pm.caller), ["Ver el resultado"]));
    assert.equal(error, null);
    assert.equal(pm.calls.length, 1, "no repair round when every source resolves");
    assert.match(out, /Proposed S002 /);
    assert.doesNotMatch(out, /fail|not exist|unknown|does not|no existe|desconocid/i);
    assert.deepEqual((await loadStory("p", "S002", s.root)).sources, [
      { type: "path", ref: "src/app.ts" },
      { type: "commit", ref: s.commit },
      { type: "story", ref: "S001" },
    ]);
  } finally {
    await s.cleanup();
  }
});

test("S002.2: a proposal citing a missing path fails the check by that path, and the repair gets it as input", async () => {
  const s = await setup();
  try {
    const pm = fakePM([
      proposal([{ type: "path", ref: "src/missing.ts" }]),
      proposal([{ type: "path", ref: "src/app.ts" }]),
    ]);
    const { error } = await run(() => proposeCommands.new(ctxFor(s, pm.caller), ["Ver el resultado"]));
    assert.equal(error, null);
    assert.equal(pm.calls.length, 2, "exactly one repair round");
    assert.match(pm.calls[1]!.prompt, /# Repair/);
    assert.match(failuresOf(pm.calls[1]!.prompt), /src\/missing\.ts/, "the failure names the missing path");
    assert.deepEqual((await loadStory("p", "S002", s.root)).sources, [{ type: "path", ref: "src/app.ts" }], "the repaired proposal is the one saved");
  } finally {
    await s.cleanup();
  }
});

test("S002.3a: a proposal citing an unknown commit hash fails the check by that hash", async () => {
  const s = await setup();
  try {
    const pm = fakePM([
      proposal([{ type: "commit", ref: UNKNOWN_COMMIT }]),
      proposal([{ type: "path", ref: "src/app.ts" }]),
    ]);
    const { error } = await run(() => proposeCommands.new(ctxFor(s, pm.caller), ["Ver el resultado"]));
    assert.equal(error, null);
    assert.equal(pm.calls.length, 2, "the unknown commit triggers the one repair");
    assert.match(failuresOf(pm.calls[1]!.prompt), new RegExp(UNKNOWN_COMMIT), "the failure names the unknown commit");
  } finally {
    await s.cleanup();
  }
});

test("S002.3b: a proposal citing an unknown story id fails the check by that id", async () => {
  const s = await setup();
  try {
    const pm = fakePM([
      proposal([{ type: "story", ref: "S099" }]),
      proposal([{ type: "story", ref: "S001" }]),
    ]);
    const { error } = await run(() => proposeCommands.new(ctxFor(s, pm.caller), ["Ver el resultado"]));
    assert.equal(error, null);
    assert.equal(pm.calls.length, 2, "the unknown story id triggers the one repair");
    assert.match(failuresOf(pm.calls[1]!.prompt), /S099/, "the failure names the unknown story id");
  } finally {
    await s.cleanup();
  }
});

test("S002.4: a source that still fails after the one repair makes story new fail, naming it, and saves nothing", async () => {
  const s = await setup();
  try {
    const pm = fakePM([
      proposal([{ type: "path", ref: "src/missing.ts" }]),
      proposal([{ type: "path", ref: "src/still-missing.ts" }]),
    ]);
    const { error } = await run(() => proposeCommands.new(ctxFor(s, pm.caller), ["Ver el resultado"]));
    assert.ok(error, "the command must report the failure");
    assert.match(error.message, /src\/still-missing\.ts/, "the failing source is reported");
    assert.equal(pm.calls.length, 2, "no further repair after the second failed check");
    assert.deepEqual((await listStories("p", s.root)).map((x) => x.id), ["S001"], "the proposal is not saved as valid");
  } finally {
    await s.cleanup();
  }
});

test("S002.4 (suggest): sources that still fail after the one repair make story suggest fail, naming them, and save nothing", async () => {
  const s = await setup();
  try {
    const pm = fakePM([
      proposal([{ type: "path", ref: "src/missing.ts" }]),
      proposal([{ type: "path", ref: "src/still-missing.ts" }]),
    ]);
    const { error } = await run(() => proposeCommands.suggest(ctxFor(s, pm.caller), []));
    assert.ok(error, "the command must report the failure");
    assert.match(error.message, /src\/still-missing\.ts/, "the failing source is reported");
    assert.equal(pm.calls.length, 2, "no further repair after the second failed check");
    assert.deepEqual((await listStories("p", s.root)).map((x) => x.id), ["S001"], "the suggestion is not saved as valid");
  } finally {
    await s.cleanup();
  }
});

test("S002.4 (changes): sources that still fail after the one repair make story changes fail, naming them, and leave the story as it was", async () => {
  const s = await setup();
  try {
    const before = await loadStory("p", "S001", s.root);
    const pm = fakePM([
      proposal([{ type: "path", ref: "src/missing.ts" }]),
      proposal([{ type: "path", ref: "src/still-missing.ts" }]),
    ]);
    const { error } = await run(() => proposeCommands.changes(ctxFor(s, pm.caller), ["S001", "cambia el criterio"]));
    assert.ok(error, "the command must report the failure");
    assert.match(error.message, /src\/still-missing\.ts/, "the failing source is reported");
    assert.equal(pm.calls.length, 2, "no further repair after the second failed check");
    const after = await loadStory("p", "S001", s.root);
    assert.equal(after.approval, before.approval, "the story is not marked as changes requested");
    assert.equal(after.changes_note, null, "the note is not saved");
    assert.deepEqual(after.sources, before.sources);
  } finally {
    await s.cleanup();
  }
});

// ---- T003: story proposals and minutes resolve sources the same way ----

/** A second git repo, so that the product spans two repos. */
function addSecondRepo(s: Fixture): string {
  const other = join(s.base, "other");
  mkdirSync(other, { recursive: true });
  writeFileSync(join(other, "README.md"), "# other\n");
  git(other, "init", "-q");
  git(other, "add", ".");
  git(other, "commit", "-q", "-m", "init");
  return other;
}

test("T003.1: a path that exists in no repo of the product fails the proposal with 'the path source does not exist'", async () => {
  const s = await setup();
  try {
    const twoRepos: Product = { ...product, paths: [s.repo, addSecondRepo(s)] };
    const pm = fakePM([
      proposal([{ type: "path", ref: "docs/missing.md" }]),
      proposal([{ type: "path", ref: "docs/missing.md" }]),
    ]);
    const { error } = await run(() => proposeCommands.new({ ...ctxFor(s, pm.caller), product: twoRepos }, ["Ver el resultado"]));
    assert.ok(error, "the proposal must fail");
    assert.match(error.message, /The path source "docs\/missing\.md" does not exist/, "the user sees that the path source does not exist");
    assert.deepEqual((await listStories("p", s.root)).map((x) => x.id), ["S001"], "the failing proposal is not saved");
  } finally {
    await s.cleanup();
  }
});

test("T003.2: a commit that exists in no repo of the product fails the proposal with 'the commit source does not exist'", async () => {
  const s = await setup();
  try {
    const twoRepos: Product = { ...product, paths: [s.repo, addSecondRepo(s)] };
    const pm = fakePM([
      proposal([{ type: "commit", ref: UNKNOWN_COMMIT }]),
      proposal([{ type: "commit", ref: UNKNOWN_COMMIT }]),
    ]);
    const { error } = await run(() => proposeCommands.new({ ...ctxFor(s, pm.caller), product: twoRepos }, ["Ver el resultado"]));
    assert.ok(error, "the proposal must fail");
    assert.match(error.message, new RegExp(`The commit source "${UNKNOWN_COMMIT}" does not exist`), "the user sees that the commit source does not exist");
    assert.deepEqual((await listStories("p", s.root)).map((x) => x.id), ["S001"], "the failing proposal is not saved");
  } finally {
    await s.cleanup();
  }
});

test("T003.3: story-propose.ts no longer defines pathExists or commitExists", () => {
  const src = readFileSync(new URL("../../core/src/story-propose.ts", import.meta.url), "utf8");
  assert.doesNotMatch(src, /\b(?:function|const|let)\s+(?:pathExists|commitExists)\b/, "the proposal check uses the shared source resolver, not its own helpers");
});
