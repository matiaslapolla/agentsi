import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import { parseArgs } from "node:util";
import {
  acceptStory,
  appendStoryLog,
  executeStoryRun,
  MERGED_RESULTS,
  queueStoryRun,
  recordStoryResult,
  rejectStory,
  type MergedResult,
  reviewStory,
  stopText,
  storyLogRef,
} from "@agentsi/core";
import { UsageError } from "../command.ts";
import type { StoryContext, StorySubcommand } from "./story.ts";

const bin = fileURLToPath(new URL("../../bin/agentsi.mjs", import.meta.url));

function storyArgs(argv: string[], usage: string): { id: string; force: boolean } {
  const { values, positionals } = parseArgs({ args: argv, allowPositionals: true, options: { product: { type: "string" }, force: { type: "boolean" } } });
  const [id, ...extra] = positionals;
  if (!id || extra.length) throw new UsageError(usage);
  return { id, force: values.force ?? false };
}

const storyId = (argv: string[], usage: string): string => storyArgs(argv, usage).id;

/** Queues the story session and starts `_exec` detached, with its output in the story log. */
async function run(ctx: StoryContext, argv: string[]): Promise<number> {
  const id = storyId(argv, "agentsi story run <id> [--product <id>]");
  const story = await queueStoryRun(ctx, id);
  await appendStoryLog(ctx.product.id, id, `[${new Date().toISOString()}] Queued ${story.session!.branch} from ${story.session!.base}.\n`);
  const child = spawn(process.execPath, [bin, "story", "_exec", id, "--product", ctx.product.id], { cwd: ctx.root, detached: true, stdio: "ignore" });
  child.unref();
  console.log(`Story session of ${id} started on ${story.session!.branch}.`);
  console.log(`Log: ${storyLogRef(ctx.product.id, id)}`);
  console.log(`Then: agentsi story review ${id}`);
  return 0;
}

async function exec(ctx: StoryContext, argv: string[]): Promise<number> {
  const id = storyId(argv, "agentsi story _exec <id> [--product <id>]");
  try {
    const story = await executeStoryRun(ctx, id);
    return story.session?.state === "ready" ? 0 : 1;
  } catch (err) {
    // The detached process has no terminal, so what would have gone to stderr goes to the story log.
    await appendStoryLog(ctx.product.id, id, `[${new Date().toISOString()}] The run crashed: ${(err as Error).stack ?? (err as Error).message}\n`);
    throw err;
  }
}

async function review(ctx: StoryContext, argv: string[]): Promise<number> {
  const id = storyId(argv, "agentsi story review <id> [--product <id>]");
  const { story, stat } = await reviewStory(ctx, id);
  const s = story.session!;
  console.log(`${story.id} ${story.title}`);
  console.log(`Session: ${s.state}, attempt ${s.attempts}, branch ${s.branch}`);
  console.log(`Worktree: ${s.worktree}`);
  if (s.tests_commit) console.log(`Tests commit: ${s.tests_commit}`);
  if (s.final_commit) console.log(`Final commit: ${s.final_commit}`);
  console.log(
    s.state === "ready"
      ? "Checks: the criterion tests failed before the work, no test file changed, the tests pass and each gate command passes."
      : s.stop
        ? `${stopText(s.stop)}\n${s.failure ?? ""}`
        : s.state === "failed"
          ? `Checks: failed.\n${s.failure ?? ""}`
          : "Checks: not done yet.",
  );
  console.log(`Log: ${s.log}`);
  if (stat) console.log(`\n${stat}`);
  console.log("");
  if (s.state === "ready") {
    console.log(`Diff: git -C ${ctx.config.repo} diff ${s.base}..${s.branch}`);
    console.log(`Merge: agentsi story accept ${id}`);
  }
  console.log(`Discard: agentsi story reject ${id}${s.state === "ready" || s.state === "failed" ? "" : " (add --force to stop a session that still runs)"}`);
  return 0;
}

async function accept(ctx: StoryContext, argv: string[]): Promise<number> {
  const id = storyId(argv, "agentsi story accept <id> [--product <id>]");
  const res = await acceptStory(ctx, id);
  console.log(`Story ${id} merged into the main branch as ${res.commit} (${res.result}).`);
  return 0;
}

async function reject(ctx: StoryContext, argv: string[]): Promise<number> {
  const { id, force } = storyArgs(argv, "agentsi story reject <id> [--force] [--product <id>]");
  await rejectStory(ctx, id, { force });
  console.log(`Story ${id} discarded: worktree and branch removed. It can run again.`);
  return 0;
}

const RESULT_USAGE = `agentsi story result <id> ${MERGED_RESULTS.join("|")} --commit <sha> [--product <id>]`;

/** Corrects the result of a story that the owner merged outside `story accept`. */
async function result(ctx: StoryContext, argv: string[]): Promise<number> {
  const { values, positionals } = parseArgs({ args: argv, allowPositionals: true, options: { product: { type: "string" }, commit: { type: "string" } } });
  const [id, kind, ...extra] = positionals;
  if (!id || !kind || extra.length || !values.commit || !MERGED_RESULTS.includes(kind as MergedResult)) throw new UsageError(RESULT_USAGE);
  const story = await recordStoryResult(ctx, id, kind as MergedResult, values.commit);
  console.log(`Story ${id} is implemented, with the result ${kind} at ${story.results.at(-1)!.commit}.`);
  return 0;
}

/** Story session subcommands (unit U3). `_exec` is the detached process that `run` starts. */
export const runCommands: Record<"run" | "review" | "accept" | "reject" | "result" | "_exec", StorySubcommand> = {
  run,
  review,
  accept,
  reject,
  result,
  _exec: exec,
};
