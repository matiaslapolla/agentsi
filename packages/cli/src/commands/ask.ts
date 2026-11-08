import { parseArgs } from "node:util";
import { askRole, totalInputTokens } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

export const ask: Command = {
  name: "ask",
  summary: "Ask one role one question",
  usage: 'agentsi ask [--product <id>] --role <role> --question "<q>"',
  async run(argv) {
    const { values } = parseArgs({
      args: argv,
      options: { product: { type: "string" }, role: { type: "string" }, question: { type: "string" } },
    });
    if (!values.role) throw new UsageError("Missing --role");
    if (!values.question) throw new UsageError("Missing --question");
    const { product, scope } = resolveProduct(values.product);
    const { answer, usage: u } = await askRole({ product: product.id, scope, role: values.role, question: values.question });
    console.log(answer);
    console.error(
      `\n[${u.role} · ${u.model} · ${(u.duration_ms / 1000).toFixed(1)}s · in ${totalInputTokens(u)} (fresh ${u.input_tokens}, cache read ${u.cache_read}, cache write ${u.cache_write}) · out ${u.output_tokens} · $${u.cost_usd.toFixed(4)}]`,
    );
  },
};
