import { parseArgs } from "node:util";
import { audit as runAudit, type AuditReport } from "@agentsi/core";
import { type Command } from "../command.ts";
import { resolveProduct } from "../context.ts";

const pct = (n: number | null): string => (n === null ? "—" : `${Math.round(n * 100)}%`);
const num = (n: number): string => (Number.isInteger(n) ? String(n) : n.toFixed(2));
const rounds = (n: number | null): string => (n === null ? "—" : num(n));

const REASON = {
  unresolved_source: "fuentes que no resuelven",
  invented_source: "fuentes inventadas",
  missing_objective: "prioridades sin objetivo válido",
  timeout: "timeouts",
  sin_datos: "\"sin datos\" repetido",
} as const;

export function renderAudit(r: AuditReport): string {
  const out: string[] = [`Auditoría de ${r.product}`, ""];

  out.push("Configuración");
  out.push(r.config.valid ? "  roster válido" : `  ${r.config.failures.length} problema(s):`);
  for (const f of r.config.failures) out.push(`    - ${f}`);
  for (const o of r.config.overdue_objectives) out.push(`  objetivo vencido: ${o.id} (${o.due}) ${o.text}`);

  const m = r.meetings;
  out.push("", `Reuniones (últimas ${m.count})`);
  if (!m.count) out.push("  sin reuniones en el registro");
  else {
    out.push(`  check a la primera: ${pct(m.first_pass_rate)} · reparadas: ${pct(m.repair_rate)} · síntesis: ${pct(m.synthesis_cost_share)} del costo · fuentes inventadas: ${m.invented_sources}`);
    for (const a of m.agents.filter((a) => a.meetings)) {
      out.push(
        `  ${a.agent}: ${a.answers}/${a.meetings} respuestas, ${a.errors} errores (${a.timeouts} timeouts), ${num(a.unresolved_sources_per_meeting)} fuentes sin resolver y ${num(a.sin_datos_per_meeting)} "sin datos" por reunión, costo medio ${a.mean_cost_usd.toFixed(2)} USD`,
      );
    }
  }

  if (!r.last_repairs) out.push("  rondas de reparación de las últimas 4 reuniones: [no data]");
  else {
    out.push("  rondas de reparación de las últimas 4 reuniones:");
    for (const l of r.last_repairs) out.push(`    ${l.minutes_id}: ${l.rounds}`);
  }

  const { baseline, since } = r.repairs;
  out.push(
    `  rondas de reparación por reunión, de las actas: primeras ${baseline.minutes.length}: ${rounds(baseline.per_meeting)} · después (${since.minutes.length}): ${rounds(since.per_meeting)}`,
  );

  out.push("", "Marcas por agente");
  const marks = Object.entries(r.marks);
  if (!marks.length) out.push("  sin prioridades");
  for (const [agent, c] of marks) out.push(`  ${agent}: ${c.accepted} aceptadas, ${c.changed} cambiadas, ${c.rejected} rechazadas, ${c.unmarked} sin marcar`);

  out.push("", "Memoria");
  for (const k of r.memory) {
    out.push(`  ${k.agent}: ${k.active} activos, ${k.retired} retirados, ${k.lessons} lecciones, ${k.never_helpful} nunca útiles, ${k.chars}/${k.max_chars} caracteres`);
  }

  out.push("", "Reflexiones");
  out.push(`  última: ${r.reflections.last ? `${r.reflections.last.id} (${r.reflections.last.created_at})` : "ninguna"} · propuestas pendientes: ${r.reflections.pending_proposals} · revertidas: ${r.reflections.reverted}`);

  out.push("", "Listo para reflexionar");
  if (!r.ready_to_reflect.length) out.push("  no hay evidencia suficiente");
  for (const a of r.ready_to_reflect) {
    out.push(`  ${a.agent}: ${a.count} ${a.failure_class ? `reuniones con ${REASON[a.failure_class]}` : "prioridades rechazadas"}`);
  }
  return out.join("\n");
}

export const audit: Command = {
  name: "audit",
  summary: "Health of the product's config, meetings, marks and memory (deterministic)",
  usage: "agentsi audit [--json] [--product <id>]",
  async run(argv) {
    const { values } = parseArgs({ args: argv, options: { product: { type: "string" }, json: { type: "boolean" } } });
    const report = await runAudit(resolveProduct(values.product).product);
    console.log(values.json ? JSON.stringify(report, null, 2) : renderAudit(report));
  },
};
