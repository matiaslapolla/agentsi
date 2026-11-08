import type { Mark, Minutes } from "./types.ts";

export const markBox = (mark: Mark | null): string =>
  mark === "accepted" ? "[x]" : mark === "changed" ? "[~]" : mark === "rejected" ? "[-]" : "[ ]";

export function renderMinutesMarkdown(m: Minutes): string {
  const out: string[] = [];
  out.push(`# Minutes ${m.id}`, "");
  out.push(`- Product: ${m.product}${m.scope ? ` (${m.scope})` : ""}`);
  out.push(`- Date: ${m.created_at}`);
  out.push(`- Question: ${m.question}`);
  out.push(`- Status: ${m.status}${m.check.repaired ? " (repaired)" : ""}`, "");

  out.push("## Priorities", "");
  if (m.priorities.length === 0) out.push("_None._", "");
  m.priorities.forEach((p, i) => {
    out.push(`- #${i} ${markBox(p.mark)} **${p.title}**${p.is_new ? " `new`" : ""} (${p.owner_role})${p.objective ? ` [${p.objective}]` : ""}`);
    out.push(`   - Why: ${p.why}`);
    for (const s of p.sources) out.push(`   - Source: ${s.type} \`${s.ref}\``);
  });
  if (m.priorities.length) out.push("");

  if (m.disagreements.length) {
    out.push("## Disagreements", "");
    for (const d of m.disagreements) {
      out.push(`- **${d.topic}**`);
      for (const pos of d.positions) out.push(`  - ${pos.role}: ${pos.view}`);
    }
    out.push("");
  }

  out.push("## Decisions to confirm", "");
  if (m.decisions.length === 0) out.push("_None._");
  for (const d of m.decisions) out.push(`- [${d.status ?? "pending"}] ${d.text} — ${d.why}${d.action ? ` → ${d.action}` : ""}${d.priorities?.length ? ` (priorities: ${d.priorities.join(", ")})` : ""}`);
  out.push("");

  out.push("## Roles", "");
  for (const [role, r] of Object.entries(m.roles)) {
    out.push(`### ${role} (${r.model})`, "", r.answer, "");
    if (r.concerns.length) out.push(...r.concerns.map((c) => `- Concern: ${c}`), "");
  }

  if (m.check.repaired && m.check.first_failures?.length) {
    out.push("## Repaired failures", "", ...m.check.first_failures.map((f) => `- ${f}`), "");
  }

  if (m.check.failures.length) {
    out.push("## Check failures", "", ...m.check.failures.map((f) => `- ${f}`), "");
  }
  return out.join("\n");
}
