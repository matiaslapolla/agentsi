import { parseArgs } from "node:util";
import { indexAll, indexProduct, type IndexResult } from "@agentsi/core";
import type { Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

export function indexLine(r: IndexResult): string {
  const skipped = r.repos.filter((x) => x.skipped).length;
  const docs = r.repos.reduce((n, x) => n + x.docs, 0);
  const commits = r.repos.reduce((n, x) => n + x.commits, 0);
  const issues = r.repos.reduce((n, x) => n + (x.issues ?? 0), 0);
  return `${r.product}: ${r.repos.length} repos (${skipped} unchanged), ${docs} docs, ${commits} commits, ${issues} issues, ${r.minutes} minutes, ${r.memory} memory, ${(r.ms / 1000).toFixed(1)}s`;
}

export const indexCommand: Command = {
  name: "index",
  summary: "Update the knowledge base index",
  usage: "agentsi index [--product <id> | --all] [--force]",
  async run(argv) {
    const { values } = parseArgs({
      args: argv,
      options: { product: { type: "string" }, all: { type: "boolean" }, force: { type: "boolean" } },
    });
    const opts = { force: values.force ?? false };
    const results = values.all ? await indexAll(opts) : [await indexProduct(resolveProduct(values.product).product.id, opts)];
    for (const r of results) console.log(indexLine(r));
  },
};
