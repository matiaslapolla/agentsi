import { parseArgs } from "node:util";
import { listStories, loadStory, dataRoot, storyConfig, storyMarkdown, type Product, type StoryConfig } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { assertNotArchived, resolveProduct } from "../context.ts";
import { proposeCommands } from "./story-propose.ts";
import { runCommands } from "./story-run.ts";

export interface StoryContext {
  product: Product;
  config: StoryConfig;
  /** AgentSI checkout that holds the workspace; tests pass a temp dir. */
  root: string;
}

/** A story subcommand gets the argv after its name, `--product` included, so its parseArgs must declare `product`. */
export type StorySubcommand = (ctx: StoryContext, argv: string[]) => Promise<number | void>;

async function list(ctx: StoryContext, argv: string[]): Promise<number> {
  const { values, positionals } = parseArgs({ args: argv, allowPositionals: true, options: { product: { type: "string" }, json: { type: "boolean" }, "show-archived": { type: "boolean" } } });
  if (positionals.length) throw new UsageError(story.usage);
  const stories = await listStories(ctx.product.id, ctx.root, { includeArchived: values["show-archived"] ?? false });
  if (values.json) {
    console.log(JSON.stringify({ product: ctx.product.id, stories }, null, 2));
    return 0;
  }
  if (!stories.length) {
    console.log(`${ctx.product.id} has no stories.`);
    return 0;
  }
  const rows = stories.map((s) => [s.id, s.status, s.approval, s.session?.state ?? "-", s.owner_role, s.archived ? `[archived] ${s.title}` : s.title]);
  const widths = rows[0]!.slice(0, -1).map((_, i) => Math.max(...rows.map((r) => r[i]!.length)));
  for (const r of rows) console.log(r.map((cell, i) => (i < widths.length ? cell.padEnd(widths[i]!) : cell)).join("  "));
  return 0;
}

async function show(ctx: StoryContext, argv: string[]): Promise<number> {
  const { values, positionals } = parseArgs({ args: argv, allowPositionals: true, options: { product: { type: "string" }, json: { type: "boolean" } } });
  const [id, ...extra] = positionals;
  if (!id || extra.length) throw new UsageError(story.usage);
  const s = await loadStory(ctx.product.id, id, ctx.root);
  console.log(values.json ? JSON.stringify(s, null, 2) : storyMarkdown(s));
  return 0;
}

export const storySubcommands: Record<string, StorySubcommand> = {
  list,
  show,
  ...proposeCommands,
  ...runCommands,
};

export const story: Command = {
  name: "story",
  summary: "Stories mode: propose, approve, run and review stories",
  usage: [
    "agentsi story list [--json] [--show-archived] [--product <id>]",
    "       agentsi story show <id> [--json] [--product <id>]",
    '       agentsi story new "<text>" [--type task] [--product <id>]',
    "       agentsi story suggest [--product <id>]",
    '       agentsi story changes <id> "<note>" [--product <id>]',
    "       agentsi story approve|discard <id> [--product <id>]",
    "       agentsi story run|review|accept <id> [--product <id>]",
    "       agentsi story reject <id> [--force] [--product <id>]",
    "       agentsi story result <id> merged_changed|merged_unchanged --commit <sha> [--product <id>]",
  ].join("\n"),
  async run(argv) {
    const [sub, ...rest] = argv;
    const handler = sub && Object.hasOwn(storySubcommands, sub) ? storySubcommands[sub] : undefined;
    if (!handler) throw new UsageError(sub ? `Unknown story subcommand "${sub}".` : "Missing story subcommand.");
    const { values } = parseArgs({ args: rest, strict: false, allowPositionals: true, options: { product: { type: "string" } } });
    const { product } = resolveProduct(typeof values.product === "string" ? values.product : undefined);
    if (sub === "run") await assertNotArchived(product);
    return handler({ product, config: storyConfig(product), root: dataRoot }, rest);
  },
};
