import { createInterface } from "node:readline/promises";
import { parseArgs } from "node:util";
import { applyReset, isNoop, planReset, type ResetPlan } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

function renderPlan(plan: ResetPlan): string {
  const id = plan.product.id;
  const lines = [plan.agent ? `Se resetea ${plan.agent} en ${id} al rol global.` : `Se resetea ${id} a los roles globales.`];
  for (const rel of plan.removals) lines.push(`  elimina  ${id}/${rel}`);
  for (const rel of Object.keys(plan.writes)) lines.push(`  edita    ${id}/${rel}`);
  if (plan.rosterBefore.join() !== plan.rosterAfter.join()) {
    lines.push(`  roster   ${plan.rosterBefore.join(", ")} -> ${plan.rosterAfter.join(", ")}`);
  }
  return lines.join("\n");
}

async function confirm(): Promise<boolean> {
  const rl = createInterface({ input: process.stdin, output: process.stdout });
  try {
    return /^(s|si|sí|y|yes)$/i.test((await rl.question("¿Aplicar? (s/N) ")).trim());
  } finally {
    rl.close();
  }
}

export const reset: Command = {
  name: "reset",
  summary: "Return an agent (or the whole product) to the global roles; --all drops brief, overlays, specialists and custom roles",
  usage: "agentsi reset <agent> [--memory] [--yes] [--product <id>]\n       agentsi reset --all [--memory] [--yes] [--product <id>]",
  async run(argv) {
    const { values, positionals } = parseArgs({
      args: argv,
      allowPositionals: true,
      options: { all: { type: "boolean" }, memory: { type: "boolean" }, yes: { type: "boolean" }, product: { type: "string" } },
    });
    const [agent, ...extra] = positionals;
    if (extra.length || Boolean(agent) === Boolean(values.all)) throw new UsageError(this.usage);
    const { product } = resolveProduct(values.product);
    const plan = await planReset({ product, agent: values.all ? null : agent!, memory: values.memory ?? false });
    if (isNoop(plan)) {
      console.log("Nada que resetear.");
      return;
    }
    console.log(renderPlan(plan));
    if (!values.yes) {
      if (!process.stdin.isTTY) {
        console.error("\nNo hay terminal para confirmar: repetí con --yes.");
        return 1;
      }
      if (!(await confirm())) {
        console.log("Cancelado.");
        return 1;
      }
    }
    const commit = await applyReset(plan);
    console.log(commit.committed ? "Reseteado y commiteado." : "Reseteado, sin commit nuevo.");
    if (commit.error) console.error(`Commit skipped: ${commit.error}`);
  },
};
