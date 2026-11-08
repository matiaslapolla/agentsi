import { parseArgs } from "node:util";
import { archiveDecision, archiveProduct, restoreProduct, archiveMinutes, archiveStory, restoreDecision, restoreMinutes, restoreStory, STORY_ID, type CommitResult } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

/** A decision id is a plain number; a minutes id is a dated name. */
const isDecisionId = (id: string) => /^\d+$/.test(id);

function make(
  name: "archive" | "restore",
  summary: string,
  apply: (product: string, id: string) => Promise<CommitResult>,
  applyDecision: (product: string, id: number) => Promise<CommitResult>,
  applyStory: (product: string, id: string) => Promise<CommitResult>,
  applyProduct: (product: string) => Promise<CommitResult>,
  past: string,
): Command {
  return {
    name,
    summary,
    usage: `agentsi ${name} [minutes-id|decision-id|story-id] [--product <id>]  (without an id: the whole product)`,
    async run(argv) {
      const { values, positionals } = parseArgs({ args: argv, allowPositionals: true, options: { product: { type: "string" } } });
      const [id] = positionals;
      if (positionals.length > 1) throw new UsageError(this.usage);
      const { product } = resolveProduct(values.product);
      if (!id) {
        const whole = await applyProduct(product.id);
        console.log(`${past} product ${product.id}.`);
        if (whole.error) console.error(`Commit skipped: ${whole.error}`);
        return;
      }
      const decision = isDecisionId(id);
      const story = STORY_ID.test(id);
      const commit = await (decision ? applyDecision(product.id, Number(id)) : story ? applyStory(product.id, id) : apply(product.id, id));
      console.log(`${past} ${decision ? "decision " : story ? "story " : ""}${id}.`);
      if (commit.error) console.error(`Commit skipped: ${commit.error}`);
    },
  };
}

export const archive = make(
  "archive",
  "Hide a minutes file, a pending decision or a story from the lists and counts; nothing is deleted",
  archiveMinutes,
  archiveDecision,
  archiveStory,
  archiveProduct,
  "Archived",
);
export const restore = make(
  "restore",
  "Bring an archived minutes file, decision or story back to the lists",
  restoreMinutes,
  restoreDecision,
  restoreStory,
  restoreProduct,
  "Restored",
);
