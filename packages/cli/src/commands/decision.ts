import { parseArgs } from "node:util";
import { addProductDecision } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

export const decision: Command = {
  name: "decision",
  summary: "Record a pending decision outside a meeting",
  usage: 'agentsi decision add --text "<text>" --why "<why>" [--action "<what to run once approved>"] [--product <id>]',
  async run(argv) {
    const [sub, ...rest] = argv;
    const { values, positionals } = parseArgs({
      args: rest,
      allowPositionals: true,
      options: { product: { type: "string" }, text: { type: "string" }, why: { type: "string" }, action: { type: "string" } },
    });
    const text = values.text?.trim();
    const why = values.why?.trim();
    if (sub !== "add" || positionals.length || !text || !why) throw new UsageError(this.usage);
    const { product } = resolveProduct(values.product);
    const id = await addProductDecision(product.id, text, why, values.action?.trim());
    console.log(`Decision ${id} pending in ${product.id}.`);
  },
};
