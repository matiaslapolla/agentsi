import { parseArgs } from "node:util";
import { snapshot as buildSnapshot } from "@agentsi/core";
import type { Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

export const snapshot: Command = {
  name: "snapshot",
  summary: "Markdown context for a meeting",
  usage: "agentsi snapshot [--product <id>]",
  async run(argv) {
    const { values } = parseArgs({ args: argv, options: { product: { type: "string" } } });
    const { product, scope } = resolveProduct(values.product);
    process.stdout.write(await buildSnapshot(product.id, scope));
  },
};
