import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { test } from "node:test";
import { archiveStory, setPool } from "@agentsi/core";
import { validateStory, type Story } from "@agentsi/core/story";
import { fixtureRoot } from "./stories-fixture";
import { changesCommand, groupStories, readStories, readStory, sessionSteps, storySentence } from "./stories";

const steps = (s: Story) => sessionSteps(s.session!).map((x) => `${x.state}:${x.status}`);

test("the fixture stories are valid", async (t) => {
  const root = await fixtureRoot(t);
  for (const s of (await readStories("agentsi", root)).stories) {
    assert.deepEqual(validateStory(s, { roster: ["pm", "dev", "qa"] }), [], s.id);
  }
});

test("readStories groups the fixtures by status, oldest id first", async (t) => {
  const { stories, error } = await readStories("agentsi", await fixtureRoot(t));
  assert.equal(error, null);
  assert.deepEqual(
    groupStories(stories).map((g) => [g.status, g.stories.map((s) => s.id)]),
    [
      ["not_implemented", ["S001", "T002"]],
      ["implemented", ["S003"]],
      ["discarded", []],
    ],
  );
});

test("readStories reports a database failure instead of throwing", async (t) => {
  const root = await fixtureRoot(t);
  setPool({ query: async () => { throw new Error("db down"); } } as never);
  const list = await readStories("agentsi", root);
  assert.deepEqual(list.stories, []);
  assert.match(list.error ?? "", /db down/);
});

test("readStory returns null for an unknown or invalid id", async (t) => {
  const root = await fixtureRoot(t);
  assert.equal((await readStory("agentsi", "S001", root))?.title, "Ver las stories en la web");
  assert.equal(await readStory("agentsi", "S099", root), null);
  assert.equal(await readStory("agentsi", "../x", root), null);
  assert.deepEqual(await readStories("nope", root), { stories: [], error: null });
});

test("sessionSteps marks the steps of a ready, a running and a failed session", async (t) => {
  const root = await fixtureRoot(t);
  const ready = (await readStory("agentsi", "S003", root))!;
  assert.deepEqual(steps(ready), ["queued:done", "tests:done", "work:done", "checks:done", "ready:done"]);
  assert.deepEqual(steps({ ...ready, session: { ...ready.session!, state: "work" } }), [
    "queued:done",
    "tests:done",
    "work:current",
    "checks:todo",
    "ready:todo",
  ]);
  const failed = (await readStory("agentsi", "T002", root))!;
  assert.deepEqual(steps(failed), ["queued:done", "tests:done", "work:done", "checks:failed", "failed:failed"]);
  assert.deepEqual(steps({ ...failed, session: { ...failed.session!, tests_commit: null, attempts: 0 } }), [
    "queued:done",
    "tests:failed",
    "work:todo",
    "checks:todo",
    "failed:failed",
  ]);
});

test("storySentence reads Como, quiero, para, and drops Como for a task", () => {
  const s = { as_a: "recepcionista", i_want: "ver", so_that: "aprobar" };
  assert.deepEqual(storySentence(s).map((p) => p.lead), ["Como", "quiero", "para"]);
  assert.deepEqual(storySentence({ ...s, as_a: null }).map((p) => p.lead), ["Quiero", "para"]);
});

/** The archive record is committed in the data root, so the root is a git repo. */
function gitRoot(root: string): string {
  const git = (...args: string[]) => execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", ...args], { cwd: root, stdio: "pipe" });
  git("init", "-q");
  git("commit", "-q", "--allow-empty", "-m", "init");
  return root;
}

test("an archived story leaves the web stories list and comes back marked when the owner asks for archived ones", async (t) => {
  const root = gitRoot(await fixtureRoot(t));
  await archiveStory("agentsi", "S001", root);
  assert.equal((await readStories("agentsi", root)).stories.some((s) => s.id === "S001"), false, "hidden by default");
  const shown = (await readStories("agentsi", root, { includeArchived: true })).stories.find((s) => s.id === "S001");
  assert.ok(shown, "listed when the owner asks for archived stories");
  assert.equal((shown as { archived?: boolean }).archived, true);
});

test("the product card counts leave out an archived story", async (t) => {
  const root = gitRoot(await fixtureRoot(t));
  const { storyCount } = await import("./inbox-data");
  const before = await storyCount("agentsi", root);
  assert.equal(before, 3, "the product card counts its stories");
  await archiveStory("agentsi", "S001", root);
  assert.equal(await storyCount("agentsi", root), before - 1, "the archived story is not counted");
});

test("changesCommand quotes the note for the shell", () => {
  assert.equal(changesCommand("S001", 'sumá "modo oscuro" y $HOME'), 'agentsi story changes S001 "sumá \\"modo oscuro\\" y \\$HOME"');
});
