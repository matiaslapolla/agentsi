import { parseArgs } from "node:util";
import { statusMarkdown } from "@agentsi/core";
import type { Command } from "../command.ts";
import { migrationPending, resolveProduct } from "../context.ts";

export const status: Command = {
  name: "status",
  summary: "Product state, pending decisions and last minutes",
  usage: "agentsi status [--product <id>]",
  async run(argv) {
    const { values } = parseArgs({ args: argv, options: { product: { type: "string" } } });
    const { product } = resolveProduct(values.product);
    if (await migrationPending()) return 1;
    process.stdout.write(await statusMarkdown(product.id));
  },
};
