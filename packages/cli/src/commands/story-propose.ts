import { parseArgs } from "node:util";
import { decideStory, proposeStory, reviseStory, suggestStories, type Story, type StoryProposeResult } from "@agentsi/core";
import { UsageError } from "../command.ts";
import type { StoryContext, StorySubcommand } from "./story.ts";

const parse = (argv: string[]) =>
  parseArgs({ args: argv, allowPositionals: true, options: { product: { type: "string" }, type: { type: "string" }, minutes: { type: "string" } } });

const line = (s: Story) => `${s.id} ${s.title} (${s.type}, owner ${s.owner_role}, ${s.criteria.length} criteria)`;

function report(res: StoryProposeResult, verb: string): void {
  for (const s of res.stories) console.log(`${verb} ${line(s)}`);
  if (!res.stories.length) console.log("The PM proposed no stories.");
  console.log(`Cost: $${res.totalCostUsd.toFixed(3)}`);
}

const usage = (text: string) => new UsageError(text);

const create: StorySubcommand = async (ctx, argv) => {
  const { values, positionals } = parse(argv);
  const [text, ...extra] = positionals;
  if (!text || extra.length) throw usage('agentsi story new "<text>" [--type task] [--minutes <id>] [--product <id>]');
  if (values.type !== undefined && values.type !== "task" && values.type !== "story") throw usage('--type must be "story" or "task".');
  report(await proposeStory(ctx, { text, type: values.type, minutes: values.minutes }), "Proposed");
  console.log("Approve it with: agentsi story approve <id>");
};

const suggest: StorySubcommand = async (ctx, argv) => {
  if (parse(argv).positionals.length) throw usage("agentsi story suggest [--product <id>]");
  report(await suggestStories(ctx), "Suggested");
};

const changes: StorySubcommand = async (ctx, argv) => {
  const [id, note, ...extra] = parse(argv).positionals;
  if (!id || !note || extra.length) throw usage('agentsi story changes <id> "<note>" [--product <id>]');
  report(await reviseStory(ctx, id, note), "Rewritten");
};

const verdict =
  (action: "approve" | "discard"): StorySubcommand =>
  async (ctx: StoryContext, argv) => {
    const [id, ...extra] = parse(argv).positionals;
    if (!id || extra.length) throw usage(`agentsi story ${action} <id> [--product <id>]`);
    const story = await decideStory(ctx, id, action);
    console.log(`${action === "approve" ? "Approved" : "Discarded"} ${line(story)}`);
  };

/** Proposal subcommands: `new`, `suggest` and `changes` call the PM; `approve` and `discard` do not. */
export const proposeCommands: Record<"new" | "suggest" | "changes" | "approve" | "discard", StorySubcommand> = {
  new: create,
  suggest,
  changes,
  approve: verdict("approve"),
  discard: verdict("discard"),
};
