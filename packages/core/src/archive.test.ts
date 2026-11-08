import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import type { Minutes, UsageRow } from "./types.ts";

// The data root is read from AGENTSI_HOME when paths.ts loads, so the test home is set before the product code loads.
const base = mkdtempSync(join(tmpdir(), "agentsi-t028-archive-"));
const home = join(base, "data");
const repo = join(base, "product");
mkdirSync(repo);
mkdirSync(join(home, "workspace", "demo"), { recursive: true });
writeFileSync(
  join(home, "agentsi.config.json"),
  JSON.stringify({ products: [{ id: "demo", name: "Demo", paths: [repo], status: "active", roles: ["pm", "dev"] }] }, null, 2) + "\n",
);
const gitEnv = { ...process.env, GIT_AUTHOR_NAME: "t", GIT_AUTHOR_EMAIL: "t@t", GIT_COMMITTER_NAME: "t", GIT_COMMITTER_EMAIL: "t@t" };
const git = (...args: string[]) => execFileSync("git", args, { cwd: home, env: gitEnv, encoding: "utf8" }).trim();
git("init", "-q");
git("add", ".");
git("commit", "-qm", "init");
process.env.AGENTSI_HOME = home;
process.env.AGENTSI_DATABASE_URL = "postgres://x:x@127.0.0.1:1/x";
Object.assign(process.env, gitEnv);

const { writeMinutes } = await import("./minutes.ts");
const { buildLogEntry } = await import("./meeting-log.ts");
const { updateReflections } = await import("./reflections.ts");
const { archiveMinutes, restoreMinutes } = await import("./archive.ts");
const { testDb } = await import("./test-db.ts");

const usage: UsageRow[] = [{ role: "pm", model: "sonnet", input_tokens: 1, output_tokens: 1, cache_read: 0, cache_write: 0, cost_usd: 0.1, duration_ms: 10 }];

const meeting: Minutes = {
  id: "2026-10-30-0900-no-commits",
  product: "demo",
  scope: null,
  question: "Do data runs still commit?",
  created_at: "2026-10-30T09:00:00.000Z",
  roles: { pm: { model: "sonnet", answer: "No.", priorities: [], concerns: [], memory_notes: [] } },
  priorities: [{ title: "Keep the data root clean", why: "The data lives in Postgres", sources: [], owner_role: "pm", mark: null, is_new: false }],
  disagreements: [],
  decisions: [],
  script: [],
  check: { passed: true, failures: [], repaired: false },
  status: "valid",
};

test("a meeting, a reflect and an archive run make no commit in the data root", async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  const head = git("rev-parse", "HEAD");
  const count = git("rev-list", "--count", "HEAD");

  const entry = buildLogEntry({ minutes: meeting, usage, agents: { pm: { prompt: "p", memory: "", model: "sonnet", error: null, duration_ms: 1, unresolved_sources: [] } }, snapshot: "S" });
  await writeMinutes(meeting, usage, { entry, snapshot: "S" });
  await updateReflections("demo", (entries) => [
    ...entries,
    { id: "r-1", product: "demo", created_at: "2026-10-30T10:00:00.000Z", trigger: "manual", signals: [], memory: [], proposal: null, status: "applied", commit: null },
  ]);
  await archiveMinutes("demo", meeting.id);
  await restoreMinutes("demo", meeting.id);
  await archiveMinutes("demo", meeting.id);

  assert.equal(git("rev-list", "--count", "HEAD"), count, "the data root has no new commit");
  assert.equal(git("rev-parse", "HEAD"), head);
  assert.equal(git("status", "--porcelain"), "", "the data root has no changed or new file either");
});

const OLD_HELPERS = /\b(minutesDir|meetingLogPath|snapshotPath|reflectionsPath|archivePath|commitPaths|commitWorkspace)\b/;
const SKIP_DIRS = new Set(["node_modules", ".next", ".git", ".worktrees", "dist", "coverage"]);
// Besides the two files the criterion names, two test files may name a helper: this one (it holds the pattern) and
// minutes.test.ts, whose criterion 1 test uses minutesDir to prove that no minutes file appears under workspace/<product>/.
const ALLOWED = new Set([
  "packages/core/src/data-import.ts",
  "packages/core/src/paths.ts",
  "packages/core/src/archive.test.ts",
  "packages/core/src/minutes.test.ts",
]);

function sources(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((e) => {
    if (e.isDirectory()) return SKIP_DIRS.has(e.name) ? [] : sources(join(dir, e.name));
    return /\.(ts|tsx|mts|mjs|js)$/.test(e.name) ? [join(dir, e.name)] : [];
  });
}

test("no code outside data-import.ts and paths.ts uses the old file path helpers or makes a data commit, and commit.ts is gone", () => {
  const codeRoot = join(dirname(fileURLToPath(import.meta.url)), "../../..");
  const found: string[] = [];
  for (const top of ["packages", "apps"]) {
    for (const file of sources(join(codeRoot, top))) {
      const rel = relative(codeRoot, file);
      if (ALLOWED.has(rel)) continue;
      readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, i) => {
          if (OLD_HELPERS.test(line)) found.push(`${rel}:${i + 1}: ${line.trim()}`);
        });
    }
  }
  assert.deepEqual(found, [], "the old path helpers and commit helpers are used nowhere else");
  assert.ok(!sources(join(codeRoot, "packages/core/src")).some((f) => f.endsWith("/commit.ts")), "packages/core/src/commit.ts is deleted");
});
