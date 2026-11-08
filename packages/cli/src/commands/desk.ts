import { parseArgs } from "node:util";
import { indexProduct, loadRegistry, migrateHint, pendingItems, pendingMigrations, statusMarkdown } from "@agentsi/core";
import type { Command } from "../command.ts";
import { resolveProduct } from "../context.ts";
import { indexLine } from "./index.ts";
import { productJson } from "./whoami.ts";

type Step<T> = ({ ok: true } & T) | { ok: false; error: string };

const errorText = (err: unknown): string => (err as Error).message || (err as { code?: string }).code || "unknown error";

async function step<T>(fn: () => Promise<T>): Promise<Step<T>> {
  try {
    return { ok: true, ...(await fn()) };
  } catch (err) {
    return { ok: false, error: errorText(err) };
  }
}

export const desk: Command = {
  name: "desk",
  summary: "Open the front desk: product, index, status and pending decisions as one JSON object",
  usage: "agentsi desk [--product <id>]",
  async run(argv) {
    const { values } = parseArgs({ args: argv, options: { product: { type: "string" } } });
    const { product, scope } = resolveProduct(values.product);
    const index = await step(async () => ({ summary: indexLine(await indexProduct(product.id)) }));
    const status = await step(async () => {
      const pending = await pendingMigrations();
      if (pending.length) throw new Error(migrateHint(pending));
      return { markdown: await statusMarkdown(product.id) };
    });
    const { items, decisions_error } = await pendingItems(product.id);
    const out = {
      ...productJson(product, scope, loadRegistry()),
      index,
      status,
      pending_decisions: items.filter((i) => i.kind === "decision"),
      ...(decisions_error ? { decisions_error } : {}),
    };
    console.log(JSON.stringify(out, null, 2));
    return index.ok && status.ok ? 0 : 1;
  },
};
