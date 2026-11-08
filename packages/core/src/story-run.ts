import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { setTimeout as sleep } from "node:timers/promises";
import { BUDGET_STOP, ClaudeError, mapUsage, runClaude, type RawResult } from "./claude.ts";
import { codeGraphMcp, reindexCodeGraph, type McpAccess } from "./code-graph.ts";
import { commitFiles } from "./exec.ts";
import { git, run } from "./exec.ts";
import { memoryForPrompt } from "./memory.ts";
import { dataRoot } from "./paths.ts";
import type { Product, StoryConfig } from "./registry.ts";
import { agentRole, loadRole, roleDirsAt } from "./roles.ts";
import { NO_MASK, nodeModulesDirs, sandboxed, sandboxWorks, storyMask, type Mask } from "./sandbox.ts";
import { archivedStoryError, hasActiveSession, isRunningSession, isStoryArchived, loadStory, saveStory, storyActionError, storyFileBase, type Story, type StoryResultKind, type StorySession, type StoryStop } from "./story.ts";
import { appendStoryLog, storyLogRef } from "./story-log.ts";
import { storyTransaction } from "./story-write.ts";

/** One `claude -p` session of a story run, with its cwd in the story worktree. */
export interface StorySessionCall {
  agent: string;
  model: string;
  effort?: string;
  system: string;
  prompt: string;
  cwd: string;
  /** `Bash(...)` rules the session may run, besides Read, Grep, Glob, Edit and Write. */
  bash: string[];
  /** `Bash(...)` rules denied on top of `STORY_DENIED_TOOLS`. */
  denyBash?: string[];
  /** Absolute paths that the Bash sandbox of the session must not write, though they are in or beside the cwd. */
  denyWrite: string[];
  /** Paths that the Bash sandbox of the session must not read, and the paths inside them that it still reads. */
  mask?: Mask;
  timeoutMs?: number;
  /** `--max-budget-usd` of the session. */
  budgetUsd: number;
  /** The code graph server, when its reindex worked. */
  mcp?: McpAccess;
}

export interface StorySessionResult {
  text: string;
  cost_usd: number;
  duration_ms: number;
  /** True when Claude Code stopped the session at its budget cap. */
  budget_stop?: boolean;
}

export type StorySessionCaller = (c: StorySessionCall) => Promise<StorySessionResult>;

export interface StoryRunContext {
  product: Product;
  config: StoryConfig;
  /** AgentSI checkout that holds the workspace; tests pass a temp dir. */
  root?: string;
  /** Defaults to `reindexCodeGraph`; tests pass a fake. */
  reindex?: typeof reindexCodeGraph;
}

const FILE_TOOLS = ["Read", "Grep", "Glob", "Edit", "Write"];
const READ_COMMANDS = ["ls", "cat", "head", "tail", "wc", "pwd"];

/** Denied even when an allow rule matches: the runner, not the session, commits, and nothing leaves the machine. */
export const STORY_DENIED_TOOLS = [
  "Bash(git:*)",
  "Bash(git push:*)",
  "Bash(gh:*)",
  "Bash(vercel:*)",
  "Bash(curl:*)",
  "Bash(wget:*)",
  "Bash(ssh:*)",
  "Bash(scp:*)",
  "Bash(rsync:*)",
  "Bash(*db:migrate*)",
  "Bash(*db:push*)",
  "Bash(*deploy*)",
  "Bash(npm publish:*)",
  "Bash(pnpm publish:*)",
  "Bash(yarn publish:*)",
  "WebFetch",
  "WebSearch",
  // git diff cannot see a change to ignored files, so the session must not touch the toolchain or the git link.
  "Edit(./node_modules/**)",
  "Edit(./**/node_modules/**)",
  "Edit(./.git)",
  "Edit(./.git/**)",
];

const SESSION_TIMEOUT_MS = 30 * 60_000;
const COMMAND_TIMEOUT_MS = 15 * 60_000;
const OUTPUT_TAIL = 4000;
const MAX_ATTEMPTS = 2;
const TRAILER = "\n\nCo-Authored-By: Claude <noreply@anthropic.com>";
const STOP_WAIT_MS = 5000;
/** QA writes and repairs the criterion tests on this model, whatever the model of its role. */
export const QA_TEST_MODEL = "sonnet";

const testPrefix = (config: StoryConfig): string => config.test.slice(0, config.test.indexOf("{files}")).trim();

/**
 * The exact rule of `command`. Claude Code 2.1.307 reads an unescaped `*` in a rule as a wildcard, so each one
 * gets a backslash. A rule with no unescaped `*` matches only the same text, backslashes included. So a session
 * cannot run a gate with a `*` as the config writes it. The runner still runs each gate itself.
 */
const exactRule = (command: string): string => command.replace(/(\\*)\*/g, (m, slashes: string) => (slashes.length % 2 ? m : `${slashes}\\*`));

/** The text of a rule inside `Bash(...)`. Claude Code removes one backslash before a backslash, `(` or `)` when it reads the rule. */
const ruleText = (rule: string): string => rule.replaceAll("\\", "\\\\").replaceAll("(", "\\(").replaceAll(")", "\\)");

/**
 * The Bash rules of a story session: the test command up to `{files}` as a prefix, the test command of the story's
 * `files` exactly, each gate command exactly, and a few read commands as prefixes.
 */
export function storyBashAllowlist(config: StoryConfig, files: string[] = []): string[] {
  const test = testPrefix(config);
  const rules = [
    ...(test ? [`${test}:*`] : []),
    ...(test && files.length ? [exactRule(testCommand(config, files))] : []),
    ...config.gate.map((g) => exactRule(g.trim())),
    ...READ_COMMANDS.map((c) => `${c}:*`),
  ];
  return [...new Set(rules)].map((r) => `Bash(${ruleText(r)})`);
}

/**
 * Deny rules for a flag after the test prefix, such as `--import ./evil.mjs`. Claude Code matches `Bash(prefix:*)`
 * as a plain string prefix, so the allow rule alone admits any flag. A flag after the files is denied only when
 * the test command ends with `{files}`, because a fixed flag after the files would match too.
 */
export function storyBashDenylist(config: StoryConfig): string[] {
  const test = testPrefix(config);
  if (!test) return [];
  const flags = ["-*", "'-*", '"-*'];
  const afterFiles = config.test.trim().endsWith("{files}") ? flags.map((f) => `* ${f}`) : [];
  return [...flags, ...afterFiles].map((f) => `Bash(${ruleText(`${test} ${f}`)})`);
}

/**
 * The Bash sandbox of a session: writes in the cwd and the temp dir, minus `denyWrite`, reads everywhere except
 * the paths that `mask` hides, no network, no unsandboxed retry, and no start without the sandbox.
 * The allowlist still decides which commands run.
 */
export function storySandboxSettings(denyWrite: string[], mask: Mask = NO_MASK): object {
  return {
    sandbox: {
      enabled: true,
      failIfUnavailable: true,
      allowUnsandboxedCommands: false,
      autoAllowBashIfSandboxed: false,
      network: { allowedDomains: [], strictAllowlist: true },
      filesystem: { denyWrite, denyRead: mask.hide, allowRead: mask.keep },
    },
  };
}

export function storySessionArgs(c: StorySessionCall): string[] {
  const args = [
    "-p",
    "--model", c.model,
    "--system-prompt", c.system,
    "--output-format", "json",
    "--no-session-persistence",
    // --safe-mode also drops the servers of --mcp-config. --restricted and --strict-mcp-config still apply.
    ...(c.mcp ? ["--mcp-config", c.mcp.config] : ["--safe-mode"]),
    "--restricted",
    "--strict-mcp-config",
    "--disable-slash-commands",
    "--permission-mode", "dontAsk",
    "--permission-prompts", "none",
    "--max-budget-usd", String(c.budgetUsd),
    `--tools=${[...FILE_TOOLS, "Bash"].join(",")}`,
    "--allowedTools", [...FILE_TOOLS, ...c.bash, ...(c.mcp?.tools ?? [])].join(","),
    "--disallowedTools", [...STORY_DENIED_TOOLS, ...(c.denyBash ?? [])].join(","),
    "--settings", JSON.stringify(storySandboxSettings(c.denyWrite, c.mask)),
  ];
  if (c.effort) args.push("--effort", c.effort);
  return args;
}

/** Maps the JSON result of a session; a budget stop is a result, any other error is not. */
export function storySessionResult(raw: RawResult, model: string): StorySessionResult {
  if (raw.is_error && raw.subtype !== BUDGET_STOP) throw new ClaudeError(raw);
  const usage = mapUsage(raw, model);
  const result: StorySessionResult = { text: String(raw.result ?? ""), cost_usd: usage.cost_usd, duration_ms: usage.duration_ms };
  return raw.subtype === BUDGET_STOP ? { ...result, budget_stop: true } : result;
}

/** The real session caller. */
export const storySession: StorySessionCaller = async (c) => {
  const raw = await runClaude({ args: storySessionArgs(c), cwd: c.cwd, prompt: c.prompt, timeoutMs: c.timeoutMs ?? SESSION_TIMEOUT_MS }).catch((err: unknown) => {
    if (err instanceof ClaudeError && err.raw.subtype === BUDGET_STOP) return err.raw;
    throw err;
  });
  return storySessionResult(raw, c.model);
};

const tail = (s: string): string => (s.length > OUTPUT_TAIL ? `…${s.slice(-OUTPUT_TAIL)}` : s);
const shellQuote = (s: string): string => `'${s.replace(/'/g, `'\\''`)}'`;
const lines = (s: string): string[] => s.split("\n").map((l) => l.trim()).filter(Boolean);

type CommandResult = { ok: boolean; output: string };
const result = (r: { ok: boolean; stdout: string; stderr: string }): CommandResult => ({ ok: r.ok, output: tail(`${r.stdout}${r.stderr}`.trim()) });

/** The install runs from the base commit, before any session, so it runs outside the sandbox. */
const install = async (command: string, cwd: string) => result(await run("sh", ["-c", command], cwd, COMMAND_TIMEOUT_MS));

/** Tests and gates run code that a session wrote, so they run in the worktree sandbox. */
const inSandbox = async (command: string, wt: string, mask: Mask) => result(await sandboxed(wt, command, COMMAND_TIMEOUT_MS, mask));

const NO_SANDBOX = "bwrap cannot start a sandbox. Story sessions run tests and gates inside bubblewrap (Linux only): install the bubblewrap package, check it with `agentsi doctor` and run again.";

/** True while the process exists; a process of another user counts. */
function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (err) {
    return (err as NodeJS.ErrnoException).code === "EPERM";
  }
}

/** Kills the process group of a detached `_exec`, which holds its claude and command children. */
async function stop(pid: number): Promise<void> {
  try {
    process.kill(-pid, "SIGKILL");
  } catch {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // gone between the check and the kill
    }
  }
  const deadline = Date.now() + STOP_WAIT_MS;
  while (alive(pid)) {
    if (Date.now() > deadline) throw new Error(`Process ${pid} did not stop.`);
    await sleep(50);
  }
}

/** Paths under .claude/ or .worktrees/ that differ from HEAD, ignored files included: Claude Code loads config from there. */
async function agentConfigChanges(wt: string): Promise<string[]> {
  const r = await git(wt, "status", "--porcelain", "--ignored", "--untracked-files=all", "--", ".claude", ".worktrees");
  if (!r.ok) throw new Error(`git status failed in ${wt}: ${r.stderr.trim()}`);
  return r.stdout.split("\n").filter(Boolean).map((l) => l.slice(3));
}

/** The session ended because `agentsi story reject` cleared it. */
class SessionGone extends Error {}

async function gitOut(cwd: string, ...args: string[]): Promise<string> {
  const r = await git(cwd, ...args);
  if (!r.ok) throw new Error(`git ${args.join(" ")} failed in ${cwd}: ${(r.stderr || r.stdout).trim()}`);
  return r.stdout.trim();
}

/** The product's main branch: the remote default branch, else `main`, else `master`. */
export async function mainBranch(repo: string): Promise<string> {
  const remote = await git(repo, "symbolic-ref", "--short", "refs/remotes/origin/HEAD");
  if (remote.ok && remote.stdout.trim()) return remote.stdout.trim().replace(/^origin\//, "");
  for (const name of ["main", "master"]) {
    if ((await git(repo, "rev-parse", "--verify", "--quiet", `refs/heads/${name}`)).ok) return name;
  }
  throw new Error(`${repo} has no main or master branch.`);
}

/** A criterion test that the owner session says contradicts its criterion. */
export interface TestDispute {
  test: string;
  reason: string;
}

/** The last `test_disputed: <test path>: <reason>` line of a session answer whose path is in `paths`, or null. */
export function testDispute(text: string, paths: string[]): TestDispute | null {
  for (const line of text.split("\n").reverse()) {
    const m = /^test_disputed:\s*(\S+?):\s+(.+)$/.exec(line.replace(/[`*]/g, "").trim());
    if (m && paths.includes(m[1]!)) return { test: m[1]!, reason: m[2]!.trim() };
  }
  return null;
}

export const testPaths = (s: Story): string[] => [...new Set(s.criteria.flatMap((c) => (c.test ? [c.test] : [])))];

const testCommand = (config: StoryConfig, files: string[]): string => config.test.replaceAll("{files}", files.map(shellQuote).join(" "));

/**
 * Checks that the story may run and records a `queued` session with its branch, worktree and base.
 * The caller then starts `executeStoryRun` in a detached process.
 */
export async function queueStoryRun(ctx: StoryRunContext, id: string): Promise<Story> {
  const root = ctx.root ?? dataRoot;
  const repo = ctx.config.repo;
  const queue = async () => {
    const story = await loadStory(ctx.product.id, id, root);
    if (await isStoryArchived(ctx.product.id, id, root)) throw new Error(archivedStoryError(id));
    const error = storyActionError(story, "run");
    if (error) throw new Error(error);
    if (!(await sandboxWorks())) throw new Error(NO_SANDBOX);
    const name = storyFileBase(story);
    const branch = `story/${name}`;
    const worktree = join(repo, ".worktrees", name);
    if (!(await git(repo, "check-ignore", "-q", `.worktrees/${name}/`)).ok) {
      throw new Error(`.worktrees/ is not gitignored in ${repo}. Add it to the product's .gitignore before a story session.`);
    }
    if ((await git(repo, "rev-parse", "--verify", "--quiet", `refs/heads/${branch}`)).ok || existsSync(worktree)) {
      throw new Error(`Branch ${branch} or worktree ${worktree} exists from an earlier session. Run agentsi story reject ${id} first.`);
    }
    const base = await gitOut(repo, "rev-parse", await mainBranch(repo));
    const session: StorySession = {
      state: "queued",
      branch,
      worktree,
      base,
      tests_commit: null,
      final_commit: null,
      attempts: 0,
      failure: null,
      log: storyLogRef(ctx.product.id, id),
      started_at: new Date().toISOString(),
      pid: null,
    };
    return saveStory(ctx.product, { ...story, session }, root);
  };
  return storyTransaction(ctx, queue);
}

/** The install runs outside the sandbox, so it runs no package scripts: a package that needs one fails the install. */
export const INSTALL: [lockfile: string, command: string][] = [
  ["pnpm-lock.yaml", "pnpm install --frozen-lockfile --prefer-offline --ignore-scripts"],
  ["package-lock.json", "npm ci --ignore-scripts"],
  ["yarn.lock", "yarn install --frozen-lockfile --ignore-scripts"],
  ["bun.lock", "bun install --frozen-lockfile --ignore-scripts"],
  ["bun.lockb", "bun install --frozen-lockfile --ignore-scripts"],
];

/** Yarn 2 and later (a `.yarnrc.yml` beside `yarn.lock`) has no `--ignore-scripts`. Its skip-build mode runs no package scripts. */
const YARN_BERRY = "yarn install --immutable --mode=skip-build";

/** The install command of the checkout `dir`, from its lockfile, or null when it has none. */
export function installCommand(dir: string): string | null {
  const found = INSTALL.find(([file]) => existsSync(join(dir, file)));
  if (!found) return null;
  return found[0] === "yarn.lock" && existsSync(join(dir, ".yarnrc.yml")) ? YARN_BERRY : found[1];
}

function storyText(s: Story): string {
  const out = [`# Story ${s.id}: ${s.title}`, ""];
  if (s.as_a) out.push(`As a ${s.as_a},`);
  out.push(`I want ${s.i_want},`, `so that ${s.so_that}.`, "", "## Criteria", "");
  s.criteria.forEach((c, i) => {
    out.push(`${i + 1}. Given ${c.given}, when ${c.when}, then ${c.then}.`);
    out.push(`   Test: ${c.test ?? "none"}. It checks: ${c.checks}`);
  });
  return out.join("\n");
}

function systemPrompt(body: string, memory: string, story: Story): string {
  return [body, "## Your memory of this product", memory || "(empty)", storyText(story)].join("\n\n");
}

const COMMON_RULES = [
  "- You work in a git worktree of the product repo. Read CLAUDE.md and CONTEXT.md at its root, if they exist, for the conventions.",
  "- Do not run git. The runner commits your changes.",
  "- Do not push, deploy, migrate a database or call a network service.",
];

function qaPrompt(s: Story, config: StoryConfig): string {
  const paths = testPaths(s);
  return [
    `Write the criterion tests of story ${s.id}, and nothing else.`,
    "",
    ...s.criteria.filter((c) => c.test).map((c) => `- ${c.test}: given ${c.given}, when ${c.when}, then ${c.then}.`),
    "",
    "- Each test checks its criterion as a user of the product sees it.",
    "- Each test must fail now, because nobody has done the work yet. Do not implement the story.",
    "- Write only the test files above. Change no other file.",
    `- Run the tests with: ${testCommand(config, paths)}`,
    ...COMMON_RULES,
  ].join("\n");
}

/** QA's one repair round of a disputed criterion test. */
function repairPrompt(s: Story, config: StoryConfig, dispute: TestDispute): string {
  return [
    `The owner session of story ${s.id} disputes the criterion test ${dispute.test}. Its reason: ${dispute.reason}`,
    "",
    ...s.criteria.filter((c) => c.test === dispute.test).map((c) => `- The criterion: given ${c.given}, when ${c.when}, then ${c.then}.`),
    "- If the test contradicts its criterion, correct the test. If the test is right, leave it as it is.",
    `- Change only ${dispute.test}. Change no other file.`,
    "- The test must still fail now, because nobody has done the work yet. Do not implement the story.",
    `- Run it with: ${testCommand(config, [dispute.test])}`,
    ...COMMON_RULES,
  ].join("\n");
}

function ownerPrompt(s: Story, config: StoryConfig, failure: string | null, note: string | null): string {
  const paths = testPaths(s);
  const out = [paths.length ? `Do the work of story ${s.id}, so that its criterion tests pass.` : `Do the work of task ${s.id}.`, ""];
  if (paths.length) {
    out.push(`- The criterion tests are: ${paths.join(", ")}. Do not change them.`);
    out.push(`- Run them with: ${testCommand(config, paths)}`);
    out.push(
      "- Before you change code, read each criterion test and run the tests.",
      "- If a test contradicts its criterion, change no file. End your answer with this line: test_disputed: <test path>: <reason>",
      "- Then QA repairs that test one time, and a new owner session does the work.",
    );
  }
  if (config.gate.length) out.push(`- These gate commands must pass: ${config.gate.join("; ")}`);
  out.push(...COMMON_RULES);
  if (note) out.push("", note);
  if (failure) out.push("", "The previous attempt failed the checks. Correct it.", "", failure);
  return out.join("\n");
}

/** The first agent of the roster whose role is `qa`. */
async function qaAgent(product: Product, root: string): Promise<string> {
  const dirs = roleDirsAt(root);
  let id: string | undefined;
  for (const a of product.roles) {
    if ((await agentRole(a, product.id, dirs)) === "qa") {
      id = a;
      break;
    }
  }
  if (!id) throw new Error(`The roster of ${product.id} has no qa agent to write the criterion tests.`);
  return id;
}

/**
 * The story session of spec §4: worktree, QA tests, the check that they fail, the owner's work, the checks,
 * and one more attempt after a failed check. Each step saves the session under the workspace lock; the end state
 * `ready` or `failed` is also committed. A reject during the run stops it at its next save.
 */
export async function executeStoryRun(ctx: StoryRunContext, id: string, caller: StorySessionCaller = storySession): Promise<Story> {
  const root = ctx.root ?? dataRoot;
  const { product, config } = ctx;
  let story = await loadStory(product.id, id, root);
  if (story.session?.state !== "queued") throw new Error(`Story ${id} has no queued story session.`);
  const started = story.session.started_at;
  // The lines go to the story_logs table in order; the end of the run waits for the last one.
  let logTail: Promise<void> = Promise.resolve();
  let logError: unknown;
  const log = (text: string): void => {
    const line = `[${new Date().toISOString()}] ${text}\n`;
    logTail = logTail.then(() => appendStoryLog(product.id, id, line)).catch((err) => void (logError ??= err));
  };
  const flushLog = async () => {
    await logTail;
    if (logError) throw logError;
  };
  const update = async (patch: Partial<StorySession>) => {
    const save = async () => {
      const fresh = await loadStory(product.id, id, root);
      if (fresh.session?.started_at !== started) throw new SessionGone(`The story session of ${id} was rejected while it ran.`);
      return saveStory(product, { ...fresh, session: { ...fresh.session, ...patch } }, root);
    };
    story = await storyTransaction(ctx, save);
  };
  const fail = async (failure: string, stop: StoryStop | null = null): Promise<Story> => {
    log(`Failed: ${failure}`);
    await update({ state: "failed", failure, ...(stop ? { stop } : {}) });
    return story;
  };
  const { branch, worktree: wt, base } = story.session;
  const paths = testPaths(story);
  const dirs = roleDirsAt(root);
  let graph: { mcp: McpAccess; rule: string } | null = null;
  let gitLink = "";
  let mask: Mask = NO_MASK;
  /** Runs one session: its answer, and its stop when Claude Code stopped it at the budget cap. */
  const session = async (agent: string, prompt: string, model?: string): Promise<{ text: string; stop: StoryStop | null }> => {
    const def = await loadRole(agent, product.id, dirs);
    log(`Session ${agent} (${model ?? def.model}) starts.`);
    const r = await caller({
      agent,
      model: model ?? def.model,
      effort: def.effort,
      system: systemPrompt(def.body, await memoryForPrompt(product.id, agent), story),
      prompt: graph ? `${prompt}\n${graph.rule}` : prompt,
      cwd: wt,
      bash: storyBashAllowlist(config, paths),
      denyBash: storyBashDenylist(config),
      denyWrite: [join(config.repo, ".git"), join(wt, ".git"), ...nodeModulesDirs(wt)],
      mask,
      budgetUsd: config.budget_usd,
      ...(graph ? { mcp: graph.mcp } : {}),
    });
    log(`Session ${agent} ends after ${Math.round(r.duration_ms / 1000)}s, $${r.cost_usd.toFixed(2)}:\n${tail(r.text)}`);
    return { text: r.text, stop: r.budget_stop ? { reason: "budget", cap_usd: config.budget_usd } : null };
  };
  const stopped = (agent: string, stop: StoryStop) =>
    fail(`Budget: the ${agent} session reached its cap of USD ${stop.cap_usd.toFixed(2)} (stories.budget_usd in the registry) and Claude Code stopped it.`, stop);
  /** Why the session's changes cannot go to git: a changed git link, or files where Claude Code loads its config. */
  const escaped = async (who: string): Promise<string | null> => {
    if (readFileSync(join(wt, ".git"), "utf8") !== gitLink) return `${who} changed the .git file of the worktree.`;
    const config = await agentConfigChanges(wt);
    return config.length ? `${who} left files in .claude/ or .worktrees/: ${config.join(", ")}` : null;
  };

  try {
    await update({ pid: process.pid });
    log(`Story session of ${id} starts on ${branch} from ${base}.`);
    if (!(await sandboxWorks())) return await fail(NO_SANDBOX);
    mask = storyMask(config.repo);
    const project = await (ctx.reindex ?? reindexCodeGraph)(config.repo, log);
    if (project) {
      const rule = `- The code graph tools (mcp__codegraph__*, project ${project}) find symbols, callers and call paths fast. Read the file to confirm before you rely on it.`;
      graph = { mcp: codeGraphMcp(), rule };
    }
    await gitOut(config.repo, "worktree", "add", "-b", branch, wt, base);
    gitLink = readFileSync(join(wt, ".git"), "utf8");
    const installs = installCommand(wt);
    if (installs) {
      log(`Install: ${installs}`);
      const r = await install(installs, wt);
      if (!r.ok) return await fail(`The install failed: ${installs}\n${r.output}`);
    }

    if (paths.length) {
      await update({ state: "tests" });
      const qa = await qaAgent(product, root);
      const { stop: qaStop } = await session(qa, qaPrompt(story, config), QA_TEST_MODEL);
      const qaEscape = await escaped("QA");
      if (qaEscape) return await fail(qaEscape);
      if (qaStop) return await stopped(qa, qaStop);
      await gitOut(wt, "add", "-A");
      const changed = lines(await gitOut(wt, "diff", "--cached", "--name-only"));
      const outside = changed.filter((f) => !paths.includes(f));
      if (outside.length) return await fail(`QA changed files outside the criterion tests: ${outside.join(", ")}`);
      const missing = paths.filter((p) => !changed.includes(p));
      if (missing.length) return await fail(`QA did not write these criterion tests: ${missing.join(", ")}`);
      const commit = await commitFiles(wt, paths, `test(${id}): criteria tests${TRAILER}`);
      if (!commit.committed) throw new Error(`The tests commit failed: ${commit.error ?? "nothing to commit"}`);
      await update({ tests_commit: await gitOut(wt, "rev-parse", "HEAD") });
      for (const file of paths) {
        const r = await inSandbox(testCommand(config, [file]), wt, mask);
        if (r.ok) return await fail(`The test ${file} passes on tests_commit; it must fail before the work.\n${r.output}`);
      }
      log("Each criterion test fails on tests_commit.");
    }

    /** QA's one repair round: the work goes, and QA may change the disputed test. The note for the owner, or the failed story. */
    const repair = async (dispute: TestDispute): Promise<Story | string> => {
      await gitOut(wt, "reset", "-q", "--hard", story.session!.tests_commit!);
      await gitOut(wt, "clean", "-fdq");
      await update({ state: "tests" });
      const qa = await qaAgent(product, root);
      const { stop } = await session(qa, repairPrompt(story, config, dispute), QA_TEST_MODEL);
      const escape = await escaped("QA");
      if (escape) return await fail(escape);
      if (stop) return await stopped(qa, stop);
      await gitOut(wt, "add", "-A");
      const changed = lines(await gitOut(wt, "diff", "--cached", "--name-only"));
      const outside = changed.filter((f) => f !== dispute.test);
      if (outside.length) return await fail(`QA changed files outside the disputed test ${dispute.test}: ${outside.join(", ")}`);
      const last = "QA repairs no test again in this run. A new dispute fails the run.";
      if (!changed.length) return `QA reviewed the disputed test ${dispute.test} and kept it as it is. The dispute was: ${dispute.reason}\n${last}`;
      const commit = await commitFiles(wt, [dispute.test], `test(${id}): repair criterion test ${dispute.test}${TRAILER}`);
      if (!commit.committed) throw new Error(`The repair commit failed: ${commit.error ?? "nothing to commit"}`);
      await update({ tests_commit: await gitOut(wt, "rev-parse", "HEAD") });
      const r = await inSandbox(testCommand(config, [dispute.test]), wt, mask);
      if (r.ok) return await fail(`The repaired test ${dispute.test} passes on tests_commit; it must fail before the work.\n${r.output}`);
      log(`QA repaired ${dispute.test}. It fails on the new tests_commit.`);
      return `QA corrected the disputed test ${dispute.test}. The dispute was: ${dispute.reason}\n${last}`;
    };

    let failure: string | null = null;
    let note: string | null = null;
    let repaired = false;
    // The disputed attempt does not use up one of the attempts.
    for (let attempt = 1; attempt <= MAX_ATTEMPTS + (repaired ? 1 : 0); attempt++) {
      await update({ state: "work", attempts: attempt, failure });
      const owner = await session(story.owner_role, ownerPrompt(story, config, failure, note));
      const ownerEscape = await escaped("The owner session");
      if (ownerEscape) return await fail(ownerEscape);
      if (owner.stop) return await stopped(story.owner_role, owner.stop);
      const dispute = story.session?.tests_commit ? testDispute(owner.text, paths) : null;
      if (dispute) {
        log(`The owner session disputes ${dispute.test}: ${dispute.reason}`);
        if (repaired) return await fail(`The owner session disputes the criterion test ${dispute.test} again, after the QA repair: ${dispute.reason}`);
        repaired = true;
        const outcome = await repair(dispute);
        if (typeof outcome !== "string") return outcome;
        failure = null;
        note = outcome;
        continue;
      }
      const commit = await commitFiles(wt, ["."], `feat(${id}): ${story.title}${attempt > 1 ? ` (attempt ${attempt})` : ""}${TRAILER}`);
      if (commit.error) throw new Error(`The work commit failed: ${commit.error}`);
      await update({ state: "checks" });
      failure = await checks(story, config, wt, mask);
      if (!failure) {
        log("Every check passes.");
        await update({ state: "ready", failure: null, final_commit: await gitOut(wt, "rev-parse", "HEAD") });
        return story;
      }
      log(`Check failed on attempt ${attempt}: ${failure}`);
    }
    return await fail(failure!);
  } catch (err) {
    try {
      if (err instanceof SessionGone) throw err;
      return await fail((err as Error).message);
    } catch (gone) {
      if (!(gone instanceof SessionGone)) throw gone;
      log(gone.message);
      return await loadStory(product.id, id, root);
    }
  } finally {
    await flushLog();
  }
}

/** The first failed check of the work, or null when every check passes. */
async function checks(story: Story, config: StoryConfig, wt: string, mask: Mask): Promise<string | null> {
  const testsCommit = story.session!.tests_commit;
  const paths = testPaths(story);
  if (testsCommit) {
    // A repair commit holds one test only, so the check compares every criterion test path.
    const touched = lines(await gitOut(wt, "diff", "--name-only", testsCommit, "HEAD"));
    const changed = paths.filter((f) => touched.includes(f));
    if (changed.length) return `The work changed files of tests_commit: ${changed.join(", ")}. Restore them; the tests are fixed.`;
  }
  if (paths.length) {
    const r = await inSandbox(testCommand(config, paths), wt, mask);
    if (!r.ok) return `The criterion tests fail:\n${r.output}`;
  }
  for (const gate of config.gate) {
    const r = await inSandbox(gate, wt, mask);
    if (!r.ok) return `The gate command "${gate}" fails:\n${r.output}`;
  }
  return null;
}

export interface StoryReview {
  story: Story;
  /** `git diff --stat base..branch` */
  stat: string;
}

export async function reviewStory(ctx: StoryRunContext, id: string): Promise<StoryReview> {
  const story = await loadStory(ctx.product.id, id, ctx.root ?? dataRoot);
  const s = story.session;
  if (!s) throw new Error(`Story ${id} has no story session.`);
  const exists = (await git(ctx.config.repo, "rev-parse", "--verify", "--quiet", `refs/heads/${s.branch}`)).ok;
  const stat = exists ? await gitOut(ctx.config.repo, "diff", "--stat", `${s.base}..${s.branch}`) : "";
  return { story, stat };
}

export interface StoryAccept {
  story: Story;
  result: StoryResultKind;
  commit: string;
}

/**
 * Merges the story branch with `--no-ff` into the main branch of the product's main checkout, records the result,
 * removes the worktree and the merged branch, and commits the story. Refuses a dirty main checkout or worktree.
 */
export async function acceptStory(ctx: StoryRunContext, id: string): Promise<StoryAccept> {
  const root = ctx.root ?? dataRoot;
  const repo = ctx.config.repo;
  const accept = async (): Promise<StoryAccept> => {
    const story = await loadStory(ctx.product.id, id, root);
    const error = storyActionError(story, "accept");
    if (error) throw new Error(error);
    const s = story.session!;
    const main = await mainBranch(repo);
    const current = await gitOut(repo, "rev-parse", "--abbrev-ref", "HEAD");
    if (current !== main) throw new Error(`The main checkout ${repo} is on ${current}, not ${main}.`);
    if (await gitOut(repo, "status", "--porcelain", "--untracked-files=no")) throw new Error(`The main checkout ${repo} has uncommitted changes.`);
    if (existsSync(s.worktree) && (await gitOut(s.worktree, "status", "--porcelain"))) {
      throw new Error(`The worktree ${s.worktree} has uncommitted changes. Commit or discard them first.`);
    }
    const head = await gitOut(repo, "rev-parse", s.branch);
    const result: StoryResultKind = head === s.final_commit ? "merged_unchanged" : "merged_changed";
    const merge = await git(repo, "merge", "--no-ff", "-m", `Merge ${s.branch}: ${story.title}`, s.branch);
    if (!merge.ok) {
      await git(repo, "merge", "--abort");
      throw new Error(`The merge of ${s.branch} failed: ${(merge.stdout + merge.stderr).trim()}`);
    }
    const commit = await gitOut(repo, "rev-parse", "HEAD");
    if (existsSync(s.worktree)) await gitOut(repo, "worktree", "remove", s.worktree);
    await gitOut(repo, "branch", "-d", s.branch);
    const saved = await saveStory(ctx.product, { ...story, status: "implemented", results: [...story.results, { result, at: new Date().toISOString(), commit }] }, root);
    return { story: saved, result, commit };
  };
  return storyTransaction(ctx, accept);
}

export type MergedResult = Exclude<StoryResultKind, "discarded">;
export const MERGED_RESULTS: MergedResult[] = ["merged_changed", "merged_unchanged"];

/**
 * Records a merge that happened outside `story accept`, as accept records it: status `implemented` and a new result
 * with the full commit hash. Refuses a commit that is not on the main branch, and a session that is not failed.
 */
export async function recordStoryResult(ctx: StoryRunContext, id: string, kind: MergedResult, commit: string): Promise<Story> {
  const root = ctx.root ?? dataRoot;
  const repo = ctx.config.repo;
  if (!MERGED_RESULTS.includes(kind)) throw new Error(`Result "${kind}" is not one of ${MERGED_RESULTS.join(", ")}.`);
  if (!/^[0-9a-f]{4,64}$/i.test(commit)) throw new Error(`Commit "${commit}" is not a commit hash.`);
  const record = async () => {
    const story = await loadStory(ctx.product.id, id, root);
    if (hasActiveSession(story)) throw new Error(`Story ${id} has a story session in state ${story.session!.state}. Run agentsi story accept ${id} or agentsi story reject ${id}.`);
    const found = await git(repo, "rev-parse", "--verify", "--quiet", `${commit}^{commit}`);
    if (!found.ok) throw new Error(`Commit ${commit} is not in ${repo}.`);
    const full = found.stdout.trim();
    const main = await mainBranch(repo);
    if (!(await git(repo, "merge-base", "--is-ancestor", full, main)).ok) throw new Error(`Commit ${commit} is not on the main branch ${main} of ${repo}.`);
    const result = { result: kind, at: new Date().toISOString(), commit: full };
    return saveStory(ctx.product, { ...story, status: "implemented", results: [...story.results, result] }, root);
  };
  return storyTransaction(ctx, record);
}

/**
 * Records a `discarded` result, removes the worktree and the branch, clears the session so the story can run again,
 * and commits the story. A session whose process still runs needs `force`, which kills that process first.
 */
export async function rejectStory(ctx: StoryRunContext, id: string, o: { force?: boolean } = {}): Promise<Story> {
  const root = ctx.root ?? dataRoot;
  const repo = ctx.config.repo;
  const reject = async () => {
    const story = await loadStory(ctx.product.id, id, root);
    const error = storyActionError(story, "reject");
    if (error) throw new Error(error);
    const s = story.session!;
    if (isRunningSession(story) && s.pid && alive(s.pid)) {
      if (!o.force) throw new Error(`The story session of ${id} is in state ${s.state} and its process ${s.pid} still runs. Wait for it, or stop it with: agentsi story reject ${id} --force`);
      await stop(s.pid);
    }
    if (existsSync(s.worktree)) await gitOut(repo, "worktree", "remove", "--force", s.worktree);
    await git(repo, "worktree", "prune");
    if ((await git(repo, "rev-parse", "--verify", "--quiet", `refs/heads/${s.branch}`)).ok) await gitOut(repo, "branch", "-D", s.branch);
    const result = { result: "discarded" as const, at: new Date().toISOString(), commit: s.final_commit };
    return saveStory(ctx.product, { ...story, session: null, results: [...story.results, result] }, root);  };
  return storyTransaction(ctx, reject);
}
