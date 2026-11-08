import { parseArgs } from "node:util";
import { reflect as runReflect, replayTable, revertReflection, type ProposalCandidate, type ReflectionEntry } from "@agentsi/core";
import { UsageError, type Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

const indent = (text: string): string[] => text.trimEnd().split("\n").map((l) => `    ${l}`);

export function renderReflection(e: ReflectionEntry): string {
  const out = [e.reverts ? `Reversión ${e.id} de ${e.reverts} en ${e.product}` : `Reflexión ${e.id} en ${e.product}`];
  if (!e.reverts) {
    const perAgent = new Map<string, number>();
    for (const s of e.signals) perAgent.set(s.agent, (perAgent.get(s.agent) ?? 0) + 1);
    out.push(`  señales: ${e.signals.length}${perAgent.size ? ` (${[...perAgent].map(([a, n]) => `${a} ${n}`).join(", ")})` : ""}`);
  }
  for (const m of e.memory) {
    const parts = [`${m.added.length} nuevos`, `${m.retired.length} retirados`, `${m.helpful.length} útiles`, `${m.harmful.length} dañinos`];
    if (m.restored?.length) parts.push(`${m.restored.length} restaurados`);
    out.push(`  ${m.agent}: ${parts.join(", ")}`);
    if (m.diff) out.push(...indent(m.diff));
    for (const d of m.dropped) out.push(`    descartado: ${d}`);
  }
  if (!e.memory.length) out.push("  sin cambios de memoria");
  return out.join("\n");
}

const OUTCOME: Record<ProposalCandidate["outcome"], string> = {
  proposed: "propuesta",
  insufficient_evidence: "evidencia insuficiente para proponer",
  deferred: "queda para la próxima reflexión (otro agente tuvo la propuesta)",
  failed: "la propuesta falló",
};

export function renderProposals(e: ReflectionEntry | null, candidates: ProposalCandidate[]): string {
  if (!candidates.length) return "Propuestas de prompt: ningún agente pasó el umbral.";
  const out = ["Propuestas de prompt:"];
  for (const c of candidates) {
    const why = c.failure_class ? `${c.failure_class} en ${c.count} reuniones` : `${c.count} prioridades rechazadas`;
    out.push(`  ${c.agent} (${why}, ${c.cases} casos, ${c.held_out} held-out): ${OUTCOME[c.outcome]}${c.note ? ` — ${c.note}` : ""}`);
  }
  const p = e?.proposal;
  if (!p) return out.join("\n");
  out.push("", `${p.agent} → ${p.target}: ${p.status === "pending" ? `decisión pendiente #${p.decision_id}` : "descartada"}`);
  if (p.rationale) out.push(`  causa: ${p.rationale}`);
  if (p.expected_effect) out.push(`  efecto esperado: ${p.expected_effect}`);
  if (p.diff) out.push(...indent(p.diff));
  out.push(...replayTable(p.replay).map((l) => `  ${l}`));
  if (p.replay.verdict.worse.length) out.push(`  empeora: ${p.replay.verdict.worse.join(", ")}`);
  out.push(`  costo de propuesta y replay: $${p.replay.cost_usd.toFixed(3)} de $${p.replay.budget_usd}`);
  if (p.status === "pending") out.push(`  Aprobar: agentsi decide ${p.decision_id} approve (o reject).`);
  return out.join("\n");
}

async function revert(argv: string[]): Promise<number> {
  const { values, positionals } = parseArgs({ args: argv, allowPositionals: true, options: { product: { type: "string" }, json: { type: "boolean" } } });
  const [id, ...extra] = positionals;
  if (!id || extra.length) throw new UsageError(reflect.usage);
  const { entry, commit } = await revertReflection(resolveProduct(values.product).product, id);
  console.log(values.json ? JSON.stringify(entry, null, 2) : renderReflection(entry!));
  if (commit.error) console.error(`Commit skipped: ${commit.error}`);
  return 0;
}

export const reflect: Command = {
  name: "reflect",
  summary: "Curate each agent's memory, then propose one prompt change gated by replay (revert: undo one)",
  usage: "agentsi reflect [--memory-only] [--json] [--product <id>]\n       agentsi reflect revert <reflection-id> [--json] [--product <id>]",
  async run(argv) {
    if (argv[0] === "revert") return revert(argv.slice(1));
    const { values, positionals } = parseArgs({
      args: argv,
      allowPositionals: true,
      options: { product: { type: "string" }, json: { type: "boolean" }, "memory-only": { type: "boolean" } },
    });
    if (positionals.length) throw new UsageError(this.usage);
    const memoryOnly = values["memory-only"] ?? false;
    const { entry, commit, candidates } = await runReflect(resolveProduct(values.product).product, { proposals: !memoryOnly });
    if (values.json) console.log(JSON.stringify({ entry, candidates }, null, 2));
    else {
      console.log(entry ? renderReflection(entry) : "No hay señales ni memoria nueva desde la última reflexión: nada que curar.");
      if (!memoryOnly) console.log(`\n${renderProposals(entry, candidates)}`);
    }
    if (commit.error) console.error(`Commit skipped: ${commit.error}`);
  },
};
