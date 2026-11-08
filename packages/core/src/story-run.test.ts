import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { test } from "node:test";
import { ClaudeError } from "./claude.ts";
import { codeGraphMcp } from "./code-graph.ts";
import { getPool } from "./db.ts";
import { git } from "./exec.ts";
import { testDb } from "./test-db.ts";
import type { Product, StoryConfig } from "./registry.ts";
import { sandboxWorks, storyMask } from "./sandbox.ts";
import { loadStory, saveStory, type Story } from "./story.ts";
import { readStoryLog } from "./story-log.ts";
import {
  acceptStory,
  executeStoryRun,
  INSTALL,
  installCommand,
  queueStoryRun,
  recordStoryResult,
  rejectStory,
  STORY_DENIED_TOOLS,
  storyBashAllowlist,
  storyBashDenylist,
  storySandboxSettings,
  storySessionArgs,
  storySessionResult,
  testDispute,
  type StoryRunContext,
  type StorySessionCall,
  type StorySessionCaller,
} from "./story-run.ts";

const PASSING = `import { test } from "node:test";\ntest("ok", () => {});\n`;
const criterionTest = (n: number) =>
  `import assert from "node:assert/strict";\nimport { test } from "node:test";\nimport { sum } from "../src/sum.mjs";\ntest("c${n}", () => assert.equal(sum(${n}, 1), ${n + 1}));\n`;
const SUM = "export const sum = (a, b) => a + b;\n";
const WRONG_SUM = "export const sum = (a, b) => a - b;\n";

const story: Story = {
  id: "S001",
  type: "story",
  slug: "sumar",
  title: "Sumar dos números",
  as_a: "usuario",
  i_want: "sumar dos números",
  so_that: "no haga la cuenta",
  criteria: [1, 2].map((n) => ({ given: `g${n}`, when: `w${n}`, then: `t${n}`, test: `tests/c${n}.test.mjs`, checks: `c${n}` })),
  owner_role: "dev",
  objective: null,
  sources: [],
  status: "not_implemented",
  approval: "approved",
  changes_note: null,
  proposed_by: "owner",
  minutes: null,
  session: null,
  results: [],
  created_at: "2026-10-21T00:00:00.000Z",
  updated_at: "2026-10-21T00:00:00.000Z",
};

function write(dir: string, file: string, content: string): void {
  mkdirSync(dirname(join(dir, file)), { recursive: true });
  writeFileSync(join(dir, file), content);
}

async function fixture(t: { after: (fn: () => void | Promise<void>) => void }, s: Partial<Story> = {}) {
  const db = await testDb();
  t.after(() => db.close());
  const root = mkdtempSync(join(tmpdir(), "agentsi-story-run-root-"));
  const repo = mkdtempSync(join(tmpdir(), "agentsi-story-run-repo-"));
  t.after(() => {
    rmSync(root, { recursive: true, force: true });
    rmSync(repo, { recursive: true, force: true });
  });
  write(root, "plugin/agents/qa.md", "---\nmodel: haiku\n---\nYou are QA.");
  write(root, "plugin/agents/dev.md", "---\nmodel: sonnet\n---\nYou are dev.");
  write(repo, ".gitignore", ".worktrees/\n");
  write(repo, "README.md", "repo\n");
  await git(repo, "init", "-q", "-b", "main");
  await git(repo, "config", "user.email", "t@example.com");
  await git(repo, "config", "user.name", "T");
  await git(repo, "add", ".");
  await git(repo, "commit", "-qm", "init");
  const product: Product = { id: "p", name: "P", paths: [repo], status: "active", roles: ["pm", "dev", "qa"] };
  // The criterion tests run inside this test file, so they must not inherit its node:test context.
  const config: StoryConfig = { repo, test: "env -u NODE_TEST_CONTEXT node --test {files}", gate: ["node --check src/sum.mjs"], budget_usd: 5 };
  const ctx: StoryRunContext = { product, config, root, reindex: async () => null };
  await saveStory(product, { ...story, ...s }, root);
  await git(root, "init", "-q");
  await git(root, "add", ".");
  await git(root, "-c", "user.name=T", "-c", "user.email=t@example.com", "commit", "-qm", "init");
  return { root, repo, ctx };
}

/**
 * A fake session: QA writes `tests`, the owner writes `work[attempt]`, each a map of file to content.
 * The calls whose 0-based index is in `budgetStops` end at their budget cap after they write.
 */
function fakeCaller(tests: Record<string, string>, work: Record<string, string>[], budgetStops: number[] = []) {
  const calls: StorySessionCall[] = [];
  const caller: StorySessionCaller = async (c) => {
    calls.push(c);
    const files = c.agent === "qa" ? tests : work[calls.filter((x) => x.agent !== "qa").length - 1] ?? {};
    for (const [file, content] of Object.entries(files)) write(c.cwd, file, content);
    const result = { text: `${c.agent} done`, cost_usd: 0.01, duration_ms: 1000 };
    return budgetStops.includes(calls.length - 1) ? { ...result, budget_stop: true } : result;
  };
  return { caller, calls };
}

/** A fake session that follows `steps` in call order: each step may check the worktree, write files and answer. */
function scriptedCaller(steps: { files?: Record<string, string>; text?: string; check?: (cwd: string) => void }[]) {
  const calls: StorySessionCall[] = [];
  const caller: StorySessionCaller = async (c) => {
    const step = steps[calls.length] ?? {};
    calls.push(c);
    step.check?.(c.cwd);
    for (const [file, content] of Object.entries(step.files ?? {})) write(c.cwd, file, content);
    return { text: step.text ?? `${c.agent} done`, cost_usd: 0.01, duration_ms: 1000 };
  };
  return { caller, calls };
}

const subjects = async (dir: string) => (await git(dir, "log", "--format=%s")).stdout.trim().split("\n");

// The runner runs tests and gates in bwrap; without it these tests cannot run.
const noSandbox = !(await sandboxWorks()) && "bwrap is missing or cannot start a sandbox here";

const goodTests = { "tests/c1.test.mjs": criterionTest(1), "tests/c2.test.mjs": criterionTest(2) };

test("the session arguments carry the deny list and no permission bypass", () => {
  const config: StoryConfig = { repo: "/r", test: "pnpm exec node --test {files}", gate: ["pnpm typecheck", "pnpm lint"], budget_usd: 5 };
  const bash = storyBashAllowlist(config);
  assert.deepEqual(bash.slice(0, 3), ["Bash(pnpm exec node --test:*)", "Bash(pnpm typecheck)", "Bash(pnpm lint)"]);
  const denyWrite = ["/r/.git", "/r/.worktrees/S001-x/node_modules"];
  const args = storySessionArgs({ agent: "dev", model: "sonnet", system: "s", prompt: "p", cwd: "/r", bash, denyWrite, budgetUsd: 2.5 });
  const value = (flag: string) => args[args.indexOf(flag) + 1]!;
  assert.equal(value("--max-budget-usd"), "2.5");
  // Claude Code 2.1.306 confines Read, Edit and Write to the cwd under --restricted, symlinks included.
  assert.ok(args.includes("--restricted"));
  assert.equal(value("--disallowedTools"), STORY_DENIED_TOOLS.join(","));
  for (const rule of ["Edit(./node_modules/**)", "Edit(./**/node_modules/**)", "Edit(./.git)"]) assert.ok(STORY_DENIED_TOOLS.includes(rule), rule);
  assert.deepEqual(JSON.parse(value("--settings")), storySandboxSettings(denyWrite));
  assert.deepEqual(storySandboxSettings(denyWrite), {
    sandbox: {
      enabled: true,
      failIfUnavailable: true,
      allowUnsandboxedCommands: false,
      autoAllowBashIfSandboxed: false,
      network: { allowedDomains: [], strictAllowlist: true },
      filesystem: { denyWrite, denyRead: [], allowRead: [] },
    },
  });
  for (const rule of ["Bash(git push:*)", "Bash(git:*)", "Bash(vercel:*)", "Bash(curl:*)", "Bash(wget:*)", "Bash(*db:migrate*)", "Bash(*db:push*)", "Bash(*deploy*)"]) {
    assert.ok(STORY_DENIED_TOOLS.includes(rule), rule);
  }
  assert.equal(value("--permission-mode"), "dontAsk");
  assert.ok(args.includes("--strict-mcp-config"));
  assert.ok(!args.some((a) => /dangerously|bypassPermissions/.test(a)));
  assert.equal(value("--allowedTools"), ["Read", "Grep", "Glob", "Edit", "Write", ...bash].join(","));
  assert.ok(args.includes("--tools=Read,Grep,Glob,Edit,Write,Bash"));
});

test("the session settings deny reads of the hidden paths and keep the kept ones readable", () => {
  const mask = { hide: ["/h/.ssh", "/h/.config", "/data"], keep: ["/h/.config/git"] };
  const args = storySessionArgs({ agent: "dev", model: "sonnet", system: "s", prompt: "p", cwd: "/r", bash: [], denyWrite: ["/r/.git"], mask, budgetUsd: 1 });
  const settings = JSON.parse(args[args.indexOf("--settings") + 1]!) as { sandbox: { filesystem: object } };
  assert.deepEqual(settings.sandbox.filesystem, { denyWrite: ["/r/.git"], denyRead: mask.hide, allowRead: mask.keep });
});

test("each install command runs no package scripts", () => {
  assert.equal(INSTALL.length, 5);
  for (const [lockfile, command] of INSTALL) assert.match(command, / --ignore-scripts(\s|$)/, lockfile);
});

test("the install command follows the lockfile, and yarn Berry skips the build scripts", (t) => {
  const dir = mkdtempSync(join(tmpdir(), "agentsi-story-run-install-"));
  t.after(() => rmSync(dir, { recursive: true, force: true }));
  assert.equal(installCommand(dir), null);
  writeFileSync(join(dir, "yarn.lock"), "");
  assert.equal(installCommand(dir), "yarn install --frozen-lockfile --ignore-scripts");
  writeFileSync(join(dir, ".yarnrc.yml"), "nodeLinker: node-modules\n");
  assert.equal(installCommand(dir), "yarn install --immutable --mode=skip-build");
  writeFileSync(join(dir, "pnpm-lock.yaml"), "");
  assert.equal(installCommand(dir), "pnpm install --frozen-lockfile --prefer-offline --ignore-scripts");
});

/**
 * The rule match of Claude Code 2.1.307, read from its bundle. The parser removes one backslash before a backslash,
 * `(` or `)`. Then `X:*` matches X or "X " plus anything, and an unescaped `*` matches any text. A `*` after a
 * backslash matches a `*`. A rule with no unescaped `*` matches the exact command, backslashes included. A deny rule wins over an allow rule.
 */
function permitted(allow: string[], deny: string[], command: string): boolean {
  const matches = (rule: string) => {
    const body = rule.slice("Bash(".length, -1).replaceAll("\\(", "(").replaceAll("\\)", ")").replaceAll("\\\\", "\\");
    if (body.endsWith(":*")) return command === body.slice(0, -2) || command.startsWith(`${body.slice(0, -2)} `);
    const parts = body.split(/(?<!(?:^|[^\\])(?:\\\\)*\\)\*/);
    if (parts.length === 1) return command === body;
    const re = parts.map((p) => p.replace(/\\([*\\])/g, "$1").replace(/[.*+?^${}()|[\]\\'"]/g, "\\$&")).join(".*");
    return new RegExp(`^${re}$`, "s").test(command);
  };
  return !deny.some(matches) && allow.some(matches);
}

test("the gate rules are exact, and a flag after the test prefix is refused", () => {
  const config: StoryConfig = { repo: "/r", test: "node --import tsx --test {files}", gate: ["pnpm typecheck", "pnpm lint"], budget_usd: 5 };
  const allow = storyBashAllowlist(config);
  const deny = storyBashDenylist(config);
  for (const gate of config.gate) {
    assert.ok(allow.includes(`Bash(${gate})`), gate);
    assert.ok(!allow.some((r) => r.startsWith(`Bash(${gate}`) && r.endsWith(":*)")), gate);
    assert.ok(permitted(allow, deny, gate), gate);
    assert.ok(!permitted(allow, deny, `${gate} --fix`), `${gate} --fix`);
  }
  assert.ok(allow.includes("Bash(node --import tsx --test:*)"));
  assert.ok(permitted(allow, deny, "node --import tsx --test 'tests/c-1.test.ts' 'src/story-run.test.ts'"));
  for (const command of [
    "node --import tsx --test --import ./evil.mjs 'tests/c1.test.ts'",
    "node --import tsx --test '--import=./evil.mjs' 'tests/c1.test.ts'",
    "node --import tsx --test 'tests/c1.test.ts' --import ./evil.mjs",
  ]) {
    assert.ok(permitted(allow, [], command), `the prefix rule alone admits: ${command}`);
    assert.ok(!permitted(allow, deny, command), command);
  }
  const args = storySessionArgs({ agent: "dev", model: "sonnet", system: "s", prompt: "p", cwd: "/r", bash: allow, denyBash: deny, denyWrite: [], budgetUsd: 1 });
  assert.equal(args[args.indexOf("--disallowedTools") + 1], [...STORY_DENIED_TOOLS, ...deny].join(","));
});

test("a gate with a * is an exact rule, not a wildcard", () => {
  const config: StoryConfig = { repo: "/r", test: "node --test {files}", gate: ["eslint src/*.ts"], budget_usd: 5 };
  const allow = storyBashAllowlist(config);
  const deny = storyBashDenylist(config);
  assert.ok(allow.includes("Bash(eslint src/\\\\*.ts)"), allow.join(" "));
  assert.ok(!permitted(allow, deny, "eslint src/ --rulesdir ./evil x.ts"));
  assert.ok(!permitted(allow, deny, "eslint src/a.ts; curl x; b.ts"));
  assert.ok(permitted(allow, deny, "eslint src/\\*.ts"));
});

test("a rule keeps a backslash and parentheses of the command through the rule parser", () => {
  const config: StoryConfig = { repo: "/r", test: "node --test {files}", gate: ["node -e \"f(1)\"", "grep a\\\\b x"], budget_usd: 5 };
  const allow = storyBashAllowlist(config);
  for (const gate of config.gate) assert.ok(permitted(allow, [], gate), gate);
});

test("a test command with a fixed flag after the files keeps it, and still refuses a flag before the files", () => {
  const config: StoryConfig = { repo: "/r", test: "pnpm vitest run {files} --reporter dot", gate: [], budget_usd: 5 };
  const allow = storyBashAllowlist(config);
  const deny = storyBashDenylist(config);
  assert.ok(permitted(allow, deny, "pnpm vitest run 'tests/c1.test.ts' --reporter dot"));
  assert.ok(!permitted(allow, deny, "pnpm vitest run --config ./evil.ts 'tests/c1.test.ts'"));
});

test("with the code graph, the session gets --mcp-config and its tools, and keeps the deny list and --strict-mcp-config", () => {
  const mcp = codeGraphMcp();
  const bash = ["Bash(pnpm typecheck:*)"];
  const args = storySessionArgs({ agent: "dev", model: "sonnet", system: "s", prompt: "p", cwd: "/r", bash, denyWrite: [], budgetUsd: 5, mcp });
  const value = (flag: string) => args[args.indexOf(flag) + 1]!;
  assert.equal(value("--mcp-config"), mcp.config);
  assert.ok(!args.includes("--safe-mode"));
  assert.ok(args.includes("--restricted") && args.includes("--strict-mcp-config"));
  assert.equal(value("--allowedTools"), ["Read", "Grep", "Glob", "Edit", "Write", ...bash, ...mcp.tools].join(","));
  assert.equal(value("--disallowedTools"), STORY_DENIED_TOOLS.join(","));
});

test("a run reindexes the product repo first and gives each session the code graph", { skip: noSandbox }, async (t) => {
  const { repo, ctx } = await fixture(t);
  const reindexed: string[] = [];
  ctx.reindex = async (path) => (reindexed.push(path), "proj-x");
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = fakeCaller(goodTests, [{ "src/sum.mjs": SUM }]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "ready", done.session?.failure ?? "");
  assert.deepEqual(reindexed, [repo]);
  for (const c of calls) {
    assert.deepEqual(c.mcp, codeGraphMcp());
    assert.match(c.prompt, /code graph tools \(mcp__codegraph__\*, project proj-x\)/);
  }
});

test("a run gives each session the mask of the repo and the deny rules of the test prefix", { skip: noSandbox }, async (t) => {
  const { repo, ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = fakeCaller(goodTests, [{ "src/sum.mjs": SUM }]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "ready", done.session?.failure ?? "");
  assert.equal(calls.length, 2);
  for (const c of calls) {
    assert.deepEqual(c.mask, storyMask(repo));
    assert.deepEqual(c.denyBash, storyBashDenylist(ctx.config));
  }
});

test("a run on a repo with a folder that it cannot read still runs, with the mask of the readable part", { skip: noSandbox }, async (t) => {
  const { repo, ctx } = await fixture(t);
  const locked = join(repo, "locked");
  mkdirSync(locked);
  chmodSync(locked, 0o000);
  // The fixture removes the repo in its own after hook, which runs first, so the folder opens here.
  const { caller, calls } = fakeCaller(goodTests, [{ "src/sum.mjs": SUM }]);
  const done = await (async () => {
    try {
      await queueStoryRun(ctx, "S001");
      return await executeStoryRun(ctx, "S001", caller);
    } finally {
      chmodSync(locked, 0o700);
    }
  })();
  assert.equal(done.session?.state, "ready", done.session?.failure ?? "");
  for (const c of calls) assert.deepEqual(c.mask, storyMask(repo));
});

test("a run reaches ready, and accept of the unchanged branch merges it as merged_unchanged", { skip: noSandbox }, async (t) => {
  const { repo, root, ctx } = await fixture(t);
  const queued = await queueStoryRun(ctx, "S001");
  assert.equal(queued.session?.state, "queued");
  assert.equal(queued.session?.base, (await git(repo, "rev-parse", "main")).stdout.trim());
  await assert.rejects(queueStoryRun(ctx, "S001"), /has a story session in state queued/);

  const { caller, calls } = fakeCaller(goodTests, [{ "src/sum.mjs": SUM }]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "ready", done.session?.failure ?? "");
  assert.equal(done.session?.attempts, 1);
  assert.deepEqual(calls.map((c) => c.agent), ["qa", "dev"]);
  // The QA role file says haiku; the criterion tests run on sonnet.
  assert.equal(calls[0]!.model, "sonnet");
  assert.match(calls[1]!.system, /^You are dev\.[\s\S]*# Story S001: Sumar dos números/);
  const wt = done.session!.worktree;
  assert.equal(wt, join(repo, ".worktrees", "S001-sumar"));
  assert.equal(calls[1]!.cwd, wt);
  assert.equal(done.session?.final_commit, (await git(wt, "rev-parse", "HEAD")).stdout.trim());
  const log = (await git(wt, "log", "--format=%s", `${done.session!.base}..HEAD`)).stdout.trim().split("\n");
  assert.deepEqual(log, ["feat(S001): Sumar dos números", "test(S001): criteria tests"]);
  assert.match(await readStoryLog("p", "S001"), /Every check passes/);

  const accepted = await acceptStory(ctx, "S001");
  assert.equal(accepted.result, "merged_unchanged");
  assert.equal(accepted.story.status, "implemented");
  assert.equal(accepted.commit, (await git(repo, "rev-parse", "main")).stdout.trim());
  assert.equal(readFileSync(join(repo, "src/sum.mjs"), "utf8"), SUM);
  assert.ok(!existsSync(wt));
  assert.equal((await git(repo, "branch", "--list", "story/*")).stdout.trim(), "");
  assert.deepEqual(await subjects(root), ["init"], "queue, ready and accept make no commit in the data root");
  assert.deepEqual((await loadStory("p", "S001", root)).results.map((r) => r.result), ["merged_unchanged"]);
});

test("accept gives merged_changed when the branch HEAD moved after final_commit", { skip: noSandbox }, async (t) => {
  const { repo, ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const done = await executeStoryRun(ctx, "S001", fakeCaller(goodTests, [{ "src/sum.mjs": SUM }]).caller);
  const wt = done.session!.worktree;
  write(wt, "src/extra.mjs", "export const x = 1;\n");
  await git(wt, "add", ".");
  await git(wt, "commit", "-qm", "owner edits");
  const accepted = await acceptStory(ctx, "S001");
  assert.equal(accepted.result, "merged_changed");
  assert.deepEqual((await loadStory("p", "S001", ctx.root)).results.map((r) => r.result), ["merged_changed"]);
  assert.ok(existsSync(join(repo, "src/extra.mjs")));
});

test("accept refuses a main checkout that is not on the main branch", { skip: noSandbox }, async (t) => {
  const { repo, ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  await executeStoryRun(ctx, "S001", fakeCaller(goodTests, [{ "src/sum.mjs": SUM }]).caller);
  await git(repo, "checkout", "-q", "-b", "other");
  await assert.rejects(acceptStory(ctx, "S001"), /is on other, not main/);
});

test("the run fails when a criterion test passes on tests_commit", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = fakeCaller({ ...goodTests, "tests/c2.test.mjs": PASSING }, []);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "failed");
  assert.match(done.session!.failure!, /The test tests\/c2\.test\.mjs passes on tests_commit/);
  assert.ok(done.session?.tests_commit);
  assert.deepEqual(calls.map((c) => c.agent), ["qa"]);
});

test("the run fails when the owner changes a test file on both attempts", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const cheat = { "src/sum.mjs": SUM, "tests/c1.test.mjs": PASSING };
  const { caller, calls } = fakeCaller(goodTests, [cheat, cheat]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "failed");
  assert.equal(done.session?.attempts, 2);
  assert.match(done.session!.failure!, /The work changed files of tests_commit: tests\/c1\.test\.mjs/);
  assert.match(calls[2]!.prompt, /The previous attempt failed the checks[\s\S]*tests\/c1\.test\.mjs/);
});

test("a failed check gets one more owner attempt, which can reach ready", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = fakeCaller(goodTests, [{ "src/sum.mjs": WRONG_SUM }, { "src/sum.mjs": SUM }]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "ready", done.session?.failure ?? "");
  assert.equal(done.session?.attempts, 2);
  assert.match(calls[2]!.prompt, /The criterion tests fail/);
});

test("each session gets the cap of the product as its budget", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  ctx.config = { ...ctx.config, budget_usd: 1.25 };
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = fakeCaller(goodTests, [{ "src/sum.mjs": SUM }]);
  await executeStoryRun(ctx, "S001", caller);
  assert.deepEqual(calls.map((c) => c.budgetUsd), [1.25, 1.25]);
});

test("a budget stop in the QA session fails the run with the cap, and the owner session does not start", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = fakeCaller(goodTests, [{ "src/sum.mjs": SUM }], [0]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "failed");
  assert.deepEqual(done.session?.stop, { reason: "budget", cap_usd: 5 });
  assert.match(done.session!.failure!, /^Budget: the qa session reached its cap of USD 5\.00/);
  assert.deepEqual(calls.map((c) => c.agent), ["qa"]);
  assert.equal(done.session?.tests_commit, null);
});

test("a budget stop in the first owner attempt fails the run, and no second attempt starts", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = fakeCaller(goodTests, [{ "src/sum.mjs": WRONG_SUM }, { "src/sum.mjs": SUM }], [1]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "failed");
  assert.equal(done.session?.attempts, 1);
  assert.deepEqual(done.session?.stop, { reason: "budget", cap_usd: 5 });
  assert.match(done.session!.failure!, /^Budget: the dev session/);
  assert.deepEqual(calls.map((c) => c.agent), ["qa", "dev"]);
});

test("a session that leaves files in .claude/ fails on that escape, even when it also stopped at its cap", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const { caller } = fakeCaller({ ...goodTests, ".claude/settings.json": "{}" }, [], [0]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "failed");
  assert.match(done.session!.failure!, /QA left files in \.claude\/ or \.worktrees\/: \.claude\/settings\.json/);
  assert.equal(done.session?.stop, undefined);
});

test("a budget stop is a session result, and any other error result still throws", () => {
  const raw = { is_error: true, subtype: "error_max_budget_usd", total_cost_usd: 5.01, duration_ms: 9000, errors: ["Reached maximum budget ($5)"] };
  assert.deepEqual(storySessionResult(raw, "sonnet"), { text: "", cost_usd: 5.01, duration_ms: 9000, budget_stop: true });
  assert.deepEqual(storySessionResult({ subtype: "success", result: "done", total_cost_usd: 0.3, duration_ms: 10 }, "sonnet"), { text: "done", cost_usd: 0.3, duration_ms: 10 });
  assert.throws(() => storySessionResult({ is_error: true, subtype: "error_max_turns" }, "sonnet"), (err: unknown) => err instanceof ClaudeError && /error_max_turns/.test(err.message));
});

test("the run fails when QA writes a file outside the criterion tests", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const done = await executeStoryRun(ctx, "S001", fakeCaller({ ...goodTests, "src/sum.mjs": SUM }, []).caller);
  assert.equal(done.session?.state, "failed");
  assert.match(done.session!.failure!, /QA changed files outside the criterion tests: src\/sum\.mjs/);
});

test("a task without tests skips QA and keeps the gate", { skip: noSandbox }, async (t) => {
  const criteria = story.criteria.map((c) => ({ ...c, test: null }));
  const { ctx } = await fixture(t, { id: "T001", type: "task", as_a: null, criteria });
  await queueStoryRun(ctx, "T001");
  const { caller, calls } = fakeCaller({}, [{}, { "src/sum.mjs": SUM }]);
  const done = await executeStoryRun(ctx, "T001", caller);
  assert.equal(done.session?.state, "ready", done.session?.failure ?? "");
  assert.equal(done.session?.tests_commit, null);
  assert.deepEqual(calls.map((c) => c.agent), ["dev", "dev"]);
  assert.match(calls[1]!.prompt, /The gate command "node --check src\/sum\.mjs" fails/);
});

test("reject records discarded, removes the worktree and the branch, and lets the story run again", { skip: noSandbox }, async (t) => {
  const { repo, ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const failed = await executeStoryRun(ctx, "S001", fakeCaller({ ...goodTests, "tests/c1.test.mjs": PASSING }, []).caller);
  await assert.rejects(queueStoryRun(ctx, "S001"), /exists from an earlier session/);
  const rejected = await rejectStory(ctx, "S001");
  assert.equal(rejected.session, null);
  assert.equal(rejected.status, "not_implemented");
  assert.deepEqual(rejected.results.map((r) => r.result), ["discarded"]);
  assert.ok(!existsSync(failed.session!.worktree));
  assert.equal((await git(repo, "branch", "--list", "story/*")).stdout.trim(), "");
  assert.equal((await queueStoryRun(ctx, "S001")).session?.state, "queued");
});

test("queue refuses a pending story and a repo that does not ignore .worktrees/", { skip: noSandbox }, async (t) => {
  const { repo, root, ctx } = await fixture(t);
  await saveStory(ctx.product, { ...story, approval: "pending" }, root);
  await assert.rejects(queueStoryRun(ctx, "S001"), /approve it before the run/);
  await saveStory(ctx.product, story, root);
  writeFileSync(join(repo, ".gitignore"), "");
  await assert.rejects(queueStoryRun(ctx, "S001"), /\.worktrees\/ is not gitignored/);
});

test("the run fails when the owner leaves files in .claude/ or changes the .git file of the worktree", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const config = await executeStoryRun(ctx, "S001", fakeCaller(goodTests, [{ "src/sum.mjs": SUM, ".claude/settings.json": "{}" }]).caller);
  assert.equal(config.session?.state, "failed");
  assert.match(config.session!.failure!, /The owner session left files in \.claude\/ or \.worktrees\/: \.claude\/settings\.json/);
  assert.equal(config.session?.attempts, 1);

  await rejectStory(ctx, "S001");
  await queueStoryRun(ctx, "S001");
  const link = await executeStoryRun(ctx, "S001", fakeCaller(goodTests, [{ "src/sum.mjs": SUM, ".git": "gitdir: /tmp/elsewhere\n" }]).caller);
  assert.match(link.session!.failure!, /The owner session changed the \.git file of the worktree/);
});

test("a reject while the run goes on stops the run at its next save", { skip: noSandbox }, async (t) => {
  const { root, ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const caller: StorySessionCaller = async (c) => {
    // What rejectStory leaves behind, written while the QA session runs.
    await saveStory(ctx.product, { ...(await loadStory("p", "S001", root)), session: null }, root);
    return fakeCaller(goodTests, []).caller(c);
  };
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session, null);
  assert.match(await readStoryLog("p", "S001"), /was rejected while it ran/);
});

test("the log text of a story run is stored in story_logs by product and story id, and no .log file is written", { skip: noSandbox }, async (t) => {
  const db = await testDb();
  t.after(() => db.close());
  await getPool().query("insert into products (id, name, paths, status) values ($1, $2, $3, $4)", ["p", "P", "[]", "active"]);
  const { root, ctx } = await fixture(t);
  // The fixture saved the story with the sync call of the file era; save it again so it is in the database.
  await saveStory(ctx.product, story, root);
  await queueStoryRun(ctx, "S001");
  const caller: StorySessionCaller = async (c) => {
    await saveStory(ctx.product, { ...(await loadStory("p", "S001", root)), session: null }, root);
    return fakeCaller(goodTests, []).caller(c);
  };
  await executeStoryRun(ctx, "S001", caller);

  const rows = ((await getPool().query("select * from story_logs")).rows as Record<string, unknown>[]).filter(
    (r) => Object.values(r).includes("p") && Object.values(r).includes("S001"),
  );
  assert.equal(rows.length, 1, "one log row for the product and story id");
  const text = Object.values(rows[0]!).find((v) => typeof v === "string" && v.includes("was rejected while it ran"));
  assert.match(String(text), /Session qa \(sonnet\) starts\.[\s\S]*was rejected while it ran/);
  assert.equal(existsSync(join(root, "workspace/p/stories/S001.log")), false);
});

test("reject refuses a running session whose process lives, and --force kills that process first", { skip: noSandbox }, async (t) => {
  const { root, ctx } = await fixture(t);
  const queued = await queueStoryRun(ctx, "S001");
  const child = spawn("sleep", ["60"], { detached: true, stdio: "ignore" });
  const exited = new Promise((resolve) => child.once("exit", resolve));
  t.after(() => child.kill("SIGKILL"));
  await saveStory(ctx.product, { ...queued, session: { ...queued.session!, state: "work", pid: child.pid! } }, root);
  await assert.rejects(rejectStory(ctx, "S001"), new RegExp(`state work and its process ${child.pid} still runs[\\s\\S]*agentsi story reject S001 --force`));
  assert.equal((await loadStory("p", "S001", root)).session?.state, "work");
  const rejected = await rejectStory(ctx, "S001", { force: true });
  await exited;
  assert.equal(rejected.session, null);
  assert.deepEqual(rejected.results.map((r) => r.result), ["discarded"]);
});

test("reject clears a running session whose process died without --force", { skip: noSandbox }, async (t) => {
  const { root, ctx } = await fixture(t);
  const queued = await queueStoryRun(ctx, "S001");
  const child = spawn("true");
  await new Promise((resolve) => child.once("exit", resolve));
  await saveStory(ctx.product, { ...queued, session: { ...queued.session!, state: "checks", pid: child.pid! } }, root);
  assert.equal((await rejectStory(ctx, "S001")).session, null);
});

test("result refuses a story with a session that is not failed, an unknown commit and a commit off the main branch", async (t) => {
  const { repo, root, ctx } = await fixture(t);
  const main = (await git(repo, "rev-parse", "main")).stdout.trim();
  const session = { state: "ready" as const, branch: "story/S001-sumar", worktree: join(repo, ".worktrees/S001-sumar"), base: main, tests_commit: null, final_commit: null, attempts: 1, failure: null, log: "l", started_at: story.created_at };
  await saveStory(ctx.product, { ...story, session }, root);
  await assert.rejects(recordStoryResult(ctx, "S001", "merged_changed", main), /has a story session in state ready/);
  await saveStory(ctx.product, { ...story, session: { ...session, state: "failed" } }, root);
  await assert.rejects(recordStoryResult(ctx, "S001", "merged_changed", "abcdef1"), /Commit abcdef1 is not in/);
  await assert.rejects(recordStoryResult(ctx, "S001", "merged_changed", "--all"), /is not a commit hash/);
  await git(repo, "checkout", "-q", "-b", "side");
  write(repo, "side.txt", "x\n");
  await git(repo, "add", ".");
  await git(repo, "commit", "-qm", "side");
  const side = (await git(repo, "rev-parse", "HEAD")).stdout.trim();
  await assert.rejects(recordStoryResult(ctx, "S001", "merged_changed", side), /is not on the main branch main/);
  assert.equal((await loadStory("p", "S001", root)).status, "not_implemented");
  const done = await recordStoryResult(ctx, "S001", "merged_unchanged", main.slice(0, 7));
  assert.equal(done.status, "implemented");
  assert.equal(done.session?.state, "failed");
  assert.deepEqual(done.results.map((r) => [r.result, r.commit]), [["merged_unchanged", main]]);
});

const WRONG_C2 = criterionTest(2).replace("sum(2, 1), 3", "sum(2, 1), 4");
const DISPUTE = "I stopped.\n`test_disputed: tests/c2.test.mjs: it expects 2 + 1 to be 4`";

test("the dispute line names a criterion test, and another line is a normal end", () => {
  const paths = ["tests/c1.test.mjs", "tests/c2.test.mjs"];
  assert.deepEqual(testDispute(DISPUTE, paths), { test: "tests/c2.test.mjs", reason: "it expects 2 + 1 to be 4" });
  assert.equal(testDispute("test_disputed: src/sum.mjs: wrong", paths), null);
  assert.equal(testDispute("All tests pass.", paths), null);
});

test("the allowlist has the exact test command of the story, and the deny rules permit it", () => {
  const config: StoryConfig = { repo: "/r", test: "node --import tsx --test {files}", gate: ["pnpm typecheck"], budget_usd: 5 };
  const files = ["apps/web/lib/listened.test.ts", "apps/web/lib/read-only.test.ts"];
  const allow = storyBashAllowlist(config, files);
  const exact = "node --import tsx --test 'apps/web/lib/listened.test.ts' 'apps/web/lib/read-only.test.ts'";
  assert.ok(allow.includes(`Bash(${exact})`), allow.join(" "));
  assert.ok(permitted(allow.filter((r) => !r.endsWith(":*)")), storyBashDenylist(config), exact));
  assert.ok(!storyBashAllowlist(config).some((r) => r.includes("'")));
});

test("a disputed test gets one QA repair round on sonnet, and the next owner session reaches ready", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  assert.notEqual(WRONG_C2, criterionTest(2));
  await queueStoryRun(ctx, "S001");
  let workGone = false;
  const { caller, calls } = scriptedCaller([
    { files: { ...goodTests, "tests/c2.test.mjs": WRONG_C2 } },
    { files: { "src/sum.mjs": SUM }, text: DISPUTE },
    { check: (cwd) => (workGone = !existsSync(join(cwd, "src/sum.mjs"))), files: { "tests/c2.test.mjs": criterionTest(2) } },
    { files: { "src/sum.mjs": SUM } },
  ]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "ready", done.session?.failure ?? "");
  assert.equal(done.session?.attempts, 2);
  assert.ok(workGone, "the repair starts from tests_commit");
  assert.deepEqual(calls.map((c) => [c.agent, c.model]), [["qa", "sonnet"], ["dev", "sonnet"], ["qa", "sonnet"], ["dev", "sonnet"]]);
  assert.match(calls[2]!.prompt, /disputes the criterion test tests\/c2\.test\.mjs\. Its reason: it expects 2 \+ 1 to be 4[\s\S]*Change only tests\/c2\.test\.mjs/);
  assert.match(calls[3]!.prompt, /QA corrected the disputed test tests\/c2\.test\.mjs/);
  assert.ok(calls[1]!.bash.includes("Bash(env -u NODE_TEST_CONTEXT node --test 'tests/c1.test.mjs' 'tests/c2.test.mjs')"));
  const wt = done.session!.worktree;
  assert.deepEqual((await git(wt, "log", "--format=%s", `${done.session!.base}..HEAD`)).stdout.trim().split("\n"), [
    "feat(S001): Sumar dos números (attempt 2)",
    "test(S001): repair criterion test tests/c2.test.mjs",
    "test(S001): criteria tests",
  ]);
  assert.equal(done.session?.tests_commit, (await git(wt, "rev-parse", "HEAD~1")).stdout.trim());
});

test("a second dispute fails the run with its reason", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = scriptedCaller([{ files: goodTests }, { text: DISPUTE }, {}, { text: DISPUTE }]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "failed");
  assert.match(done.session!.failure!, /disputes the criterion test tests\/c2\.test\.mjs again, after the QA repair: it expects 2 \+ 1 to be 4/);
  assert.match(calls[3]!.prompt, /QA reviewed the disputed test tests\/c2\.test\.mjs and kept it as it is/);
  assert.equal(calls.length, 4);
});

test("a QA repair that changes another file fails the run", { skip: noSandbox }, async (t) => {
  const { ctx } = await fixture(t);
  await queueStoryRun(ctx, "S001");
  const { caller, calls } = scriptedCaller([{ files: goodTests }, { text: DISPUTE }, { files: { "tests/c1.test.mjs": PASSING } }]);
  const done = await executeStoryRun(ctx, "S001", caller);
  assert.equal(done.session?.state, "failed");
  assert.match(done.session!.failure!, /QA changed files outside the disputed test tests\/c2\.test\.mjs: tests\/c1\.test\.mjs/);
  assert.equal(calls.length, 3);
});
