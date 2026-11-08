import { resolve } from "node:path";
import { parseArgs } from "node:util";
import { applyPlan, describeRoster, loadAgent, planApply, registerableProduct, storiesChanged, validateRoster, type ApplyPlan, type StoryConfigBlock } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

const STATUS_LABEL = { new: "nuevo", changed: "cambiado", removed: "eliminado" } as const;

const storiesLabel = (s: StoryConfigBlock | undefined): string =>
  s ? `test "${s.test}", gate ${s.gate.map((g) => `"${g}"`).join(", ")}${s.repo ? `, repo ${s.repo}` : ""}` : "apagado";

function renderPlan(plan: ApplyPlan): string {
  const { id } = plan.product;
  const lines = [`${plan.register ? "Se registra y se configura" : "Se configura"} ${id}.`, ""];
  lines.push(`Roster: ${plan.rosterBefore.join(", ") || "(ninguno)"} -> ${plan.rosterAfter.join(", ")}`);
  const added = plan.rosterAfter.filter((a) => !plan.rosterBefore.includes(a));
  const removed = plan.rosterBefore.filter((a) => !plan.rosterAfter.includes(a));
  if (added.length) lines.push(`  agregados: ${added.join(", ")}`);
  if (removed.length) lines.push(`  quitados: ${removed.join(", ")}`);
  if (storiesChanged(plan)) lines.push(`Stories: ${storiesLabel(plan.storiesBefore)} -> ${storiesLabel(plan.storiesAfter)}`);
  lines.push("", plan.changes.length ? "Base de datos (brief y overlays):" : "Base de datos (brief y overlays): sin cambios.");
  for (const c of plan.changes) {
    lines.push(`  ${STATUS_LABEL[c.status]}  ${id}/${c.path}`);
    if (c.diff) lines.push(...c.diff.split("\n").map((l) => `    ${l}`));
  }
  return lines.join("\n");
}

async function apply(argv: string[]): Promise<number> {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: {
      product: { type: "string" },
      register: { type: "boolean" },
      name: { type: "string" },
      path: { type: "string" },
      yes: { type: "boolean" },
    },
  });
  const [dir, ...extra] = positionals;
  if (!dir || extra.length) throw new UsageError(roles.usage);
  const register = values.register ?? false;
  let product;
  if (register) {
    if (!values.product) throw new UsageError("--register needs --product <id>, --name and --path.");
    product = registerableProduct(values.product, values.name, values.path);
  } else {
    product = resolveProduct(values.product).product;
  }
  const stageDir = resolve(dir);
  const { plan, failures } = await planApply({ stageDir, product, register });
  if (!plan) {
    console.error("La etapa no es válida:");
    for (const f of failures) console.error(`  - ${f}`);
    return 1;
  }
  console.log(renderPlan(plan));
  if (!values.yes) {
    console.log("\nSimulacro: no se escribió nada. Repetí con --yes para aplicar.");
    return 0;
  }
  const commit = await applyPlan(plan, stageDir);
  console.log(commit.committed ? "\nAplicado y commiteado." : "\nAplicado, sin commit nuevo.");
  if (commit.error) console.error(`Commit skipped: ${commit.error}`);
  return 0;
}

async function validate(argv: string[]): Promise<number> {
  const { values } = parseArgs({ args: argv, options: { product: { type: "string" }, json: { type: "boolean" } } });
  const { product } = resolveProduct(values.product);
  const failures = await validateRoster(product);
  if (values.json) console.log(JSON.stringify({ product: product.id, valid: failures.length === 0, failures }, null, 2));
  else if (!failures.length) console.log(`${product.id}: el roster es válido (${product.roles.length} agentes).`);
  else {
    console.error(`${product.id}: ${failures.length} problema(s):`);
    for (const f of failures) console.error(`  - ${f}`);
  }
  return failures.length ? 1 : 0;
}

async function list(argv: string[]): Promise<number> {
  const { values, positionals } = parseArgs({
    args: argv,
    allowPositionals: true,
    options: { product: { type: "string" }, json: { type: "boolean" } },
  });
  if (positionals.length > 1) throw new UsageError(roles.usage);
  const { product } = resolveProduct(values.product);
  const [agent] = positionals;
  if (agent) {
    const def = await loadAgent(agent, product.id);
    if (values.json) console.log(JSON.stringify(def, null, 2));
    else console.log(`# ${def.id} (rol ${def.role}, ${def.source}, ${def.model}${def.effort ? `, esfuerzo ${def.effort}` : ""})\n\n${def.body}`);
    return 0;
  }
  const entries = await describeRoster(product);
  if (values.json) {
    console.log(JSON.stringify({ product: product.id, roster: entries }, null, 2));
    return 0;
  }
  const col = (pick: (e: (typeof entries)[number]) => string) => Math.max(...entries.map((e) => pick(e).length));
  const [idW, roleW, srcW] = [col((e) => e.id), col((e) => e.role ?? ""), col((e) => e.source ?? "")];
  console.log(`Roster de ${product.id}:`);
  for (const e of entries) {
    const detail = e.error
      ? `ERROR ${e.error}`
      : `${e.role!.padEnd(roleW)}  ${e.source!.padEnd(srcW)}  ${e.model}${e.effort ? `  esfuerzo ${e.effort}` : ""}`;
    console.log(`  ${e.id.padEnd(idW)}  ${detail}`);
  }
  return 0;
}

export const roles: Command = {
  name: "roles",
  summary: "Roster of the product (validate: check it; apply: install a staged brief and roster)",
  usage:
    "agentsi roles [agent] [--json] [--product <id>]\n       agentsi roles validate [--json] [--product <id>]\n       agentsi roles apply <staging-dir> [--yes] [--register --product <id> --name <n> --path <abs>]",
  async run(argv) {
    const [sub, ...rest] = argv;
    if (sub === "validate") return validate(rest);
    if (sub === "apply") return apply(rest);
    return list(argv);
  },
};
