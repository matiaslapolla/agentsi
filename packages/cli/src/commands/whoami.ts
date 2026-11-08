import { parseArgs } from "node:util";
import { detectProduct, isProductArchived, loadRegistry, ownerOf, resolveDataRoot, type Product, type Registry } from "@agentsi/core";
import type { Command } from "../command.ts";

/** The product JSON of `agentsi whoami`, which `agentsi desk` also prints. */
export const productJson = (product: Product, scope: string | null, registry: Registry) =>
  ({ product: product.id, name: product.name, scope, roles: product.roles, paths: product.paths, owner: ownerOf(registry) });

export const whoami: Command = {
  name: "whoami",
  summary: "Detect the product of a directory",
  usage: "agentsi whoami [--cwd <dir>] [--list] [--home]",
  async run(argv) {
    const { values } = parseArgs({ args: argv, options: { cwd: { type: "string" }, list: { type: "boolean" }, home: { type: "boolean" } } });
    if (values.home) {
      const { root, from } = resolveDataRoot();
      console.log(`${root}  (${from})`);
      return;
    }
    if (values.list) {
      for (const p of loadRegistry().products) console.log(`${p.id}  ${p.name}  ${p.status}  ${p.paths.join(",")}${(await isProductArchived(p.id)) ? "  (archived)" : ""}`);
      return;
    }
    const registry = loadRegistry();
    const found = detectProduct(values.cwd ?? process.cwd(), registry);
    if (!found) return 2;
    const { product, scope } = found;
    console.log(JSON.stringify(productJson(product, scope, registry), null, 2));
  },
};
