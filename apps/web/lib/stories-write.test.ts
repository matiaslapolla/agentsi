import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import path from "node:path";
import { test } from "node:test";
import { saveStory, type Product } from "@agentsi/core";
import { readStories, readStory } from "./stories";
import { fixtureRoot } from "./stories-fixture";
import { archiveStory, changeStory, restoreStory } from "./stories-write";

const product: Product = { id: "agentsi", name: "AgentSI", paths: [], status: "active", roles: ["pm", "dev", "qa"] };

/** A data root that is a git repo, as the owner keeps it: the archive record is committed there. */
async function repo(t: Parameters<typeof fixtureRoot>[0]): Promise<string> {
  const root = await fixtureRoot(t);
  const git = (...args: string[]) =>
    execFileSync("git", ["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false", ...args], { cwd: root, stdio: "pipe" });
  git("init", "-q");
  git("commit", "-q", "--allow-empty", "-m", "init");
  return root;
}

const commits = (root: string) => execFileSync("git", ["rev-list", "--count", "HEAD"], { cwd: root, encoding: "utf8" }).trim();
const dirty = (root: string) => execFileSync("git", ["status", "--porcelain"], { cwd: root, encoding: "utf8" }).trim();

test("archiving a story with a running session from the web is refused and the story stays active", async (t) => {
  const root = await repo(t);
  const session = {
    state: "work" as const,
    branch: "story/S001-ver-las-stories-en-la-web",
    worktree: path.join(root, ".worktrees", "S001-ver-las-stories-en-la-web"),
    base: "abc",
    tests_commit: "def",
    final_commit: null,
    attempts: 1,
    failure: null,
    log: "story_logs agentsi/S001",
    started_at: "2026-10-21T00:00:00.000Z",
    pid: null,
  };
  await saveStory(product, { ...(await readStory("agentsi", "S001", root))!, approval: "approved", session }, root);
  await assert.rejects(archiveStory(product, "S001", root), /session.*running|running.*session/i);
  assert.equal((await readStory("agentsi", "S001", root))?.session?.state, "work", "the session is not touched");
  assert.ok((await readStories("agentsi", root)).stories.some((s) => s.id === "S001"), "the story stays in the list");
});

test("restoring an archived story from the web puts it back in the list with its previous state", async (t) => {
  const root = await repo(t);
  const before = (await readStory("agentsi", "S001", root))!;
  await archiveStory(product, "S001", root);
  await restoreStory(product, "S001", root);
  const after = (await readStory("agentsi", "S001", root))!;
  assert.deepEqual(
    [after.status, after.approval, after.session, after.results],
    [before.status, before.approval, before.session, before.results],
    "the story comes back in its previous state",
  );
  assert.ok((await readStories("agentsi", root)).stories.some((s) => s.id === "S001"), "back in the list");
});

test("approve changes the stored story and makes no commit", async (t) => {
  const root = await repo(t);
  await changeStory(product, "S001", "approve", "", root);
  assert.equal((await readStory("agentsi", "S001", root))?.approval, "approved");
  assert.equal(commits(root), "1");
  assert.equal(dirty(root), "");
});

test("changes records the note", async (t) => {
  const root = await repo(t);
  await changeStory(product, "S001", "changes", "  sumá un criterio de modo oscuro ", root);
  const s = (await readStory("agentsi", "S001", root))!;
  assert.equal(s.approval, "changes_requested");
  assert.equal(s.changes_note, "sumá un criterio de modo oscuro");
});

test("discard changes the status", async (t) => {
  const root = await repo(t);
  await changeStory(product, "S001", "discard", "", root);
  assert.equal((await readStory("agentsi", "S001", root))?.status, "discarded");
});

test("an illegal action or an empty note throws and writes nothing", async (t) => {
  const root = await repo(t);
  await assert.rejects(changeStory(product, "T002", "approve", "", root));
  await assert.rejects(changeStory(product, "S003", "discard", "", root));
  await assert.rejects(changeStory(product, "T002", "discard", "", root), /Run agentsi story reject T002 first/);
  await assert.rejects(changeStory(product, "S001", "changes", "  ", root));
  await assert.rejects(changeStory(product, "S099", "approve", "", root), /Unknown story/);
  assert.equal((await readStory("agentsi", "S001", root))?.approval, "pending");
  assert.equal(dirty(root), "");
});
