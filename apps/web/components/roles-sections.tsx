import { ChevronRight } from "lucide-react";
import { CliHint, DecideHint } from "@/components/cli-hint";
import { DiffView } from "@/components/diff-view";
import { Empty, SectionTitle } from "@/components/bits";
import { RoleAvatar, RoleTag } from "@/components/roster";
import { Badge } from "@/components/ui/badge";
import { dateOf, usd } from "@/lib/format";
import { canRevert } from "@agentsi/core/reflections";
import type { AuditReport, Brief, Loaded, Proposal, ReflectionEntry, ReplayReport, RosterRow } from "@/lib/roles-data";
import { isPast, percent } from "@/lib/roles-view";
import { roleMeta } from "@/lib/roles";

const num = (n: number): string => (Number.isInteger(n) ? String(n) : n.toFixed(2));

function Failed({ what, error }: { what: string; error: string }) {
  return (
    <Empty className="text-status-neglected">
      No se pudo leer {what}: <span className="mono text-xs">{error}</span>
    </Empty>
  );
}

function Pre({ children, label }: { children: string; label: string }) {
  return (
    <div>
      <p className="label mb-1">{label}</p>
      <pre className="mono max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-md border border-border bg-muted p-3 text-xs leading-5">
        {children}
      </pre>
    </div>
  );
}

export function RosterSection({ roster }: { roster: Loaded<RosterRow[]> }) {
  return (
    <section id="roster" className="mb-14 scroll-mt-20 md:scroll-mt-6">
      <SectionTitle aside={roster.ok ? roster.data.length : null}>Roster</SectionTitle>
      {!roster.ok ? (
        <Failed what="el roster" error={roster.error} />
      ) : roster.data.length === 0 ? (
        <Empty>Este producto no tiene agentes en el roster.</Empty>
      ) : (
        <ul className="divide-y divide-border border-y border-border">
          {roster.data.map((a) => (
            <li key={a.id} className="py-4">
              <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
                <RoleAvatar role={a.id} size={28} />
                <span className="font-semibold">{roleMeta(a.id).label}</span>
                <span className="mono text-xs text-muted-foreground">{a.id}</span>
                {a.source && <Badge tone={a.source === "global" ? "neutral" : "accent"}>{a.source}</Badge>}
                {a.model && (
                  <span className="mono text-xs text-muted-foreground">
                    {a.model}
                    {a.effort && ` · ${a.effort}`}
                  </span>
                )}
              </div>
              {a.error && <p role="alert" className="mt-2 text-sm text-status-neglected">{a.error}</p>}
              <p className="mt-2 max-w-[68ch] text-[15px] text-muted-foreground">
                {a.why ?? <span className="text-muted-foreground">El brief no dice para qué sirve este agente.</span>}
              </p>
              {(a.overlay || a.prompt) && (
                <details className="mt-3">
                  <summary className="inline-flex items-center gap-2 text-sm text-muted-foreground">
                    <ChevronRight className="chev size-4 text-muted-foreground transition-transform" />
                    Overlay y prompt efectivo
                  </summary>
                  <div className="mt-3 space-y-3">
                    {a.overlay ? (
                      <Pre label="Overlay">{a.overlay}</Pre>
                    ) : (
                      <p className="text-sm text-muted-foreground">Sin overlay: usa el prompt global.</p>
                    )}
                    {a.prompt && <Pre label="Prompt efectivo">{a.prompt}</Pre>}
                  </div>
                </details>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

export function BriefSection({ brief }: { brief: Brief | null }) {
  return (
    <section id="brief" className="mb-14 scroll-mt-20 md:scroll-mt-6">
      <SectionTitle aside={brief?.updated_at ? brief.updated_at.slice(0, 10) : null}>Brief</SectionTitle>
      {!brief ? (
        <Empty>
          Este producto todavía no tiene brief. Corré <span className="mono">/agentsi setup</span> dentro del producto.
        </Empty>
      ) : (
        <div className="space-y-6 border-y border-border py-5">
          <dl className="grid gap-x-8 gap-y-3 sm:grid-cols-[auto_minmax(0,1fr)]">
            <dt className="label">Qué es</dt>
            <dd className="max-w-[68ch]">{brief.product.what}</dd>
            <dt className="label">Para quién</dt>
            <dd className="max-w-[68ch]">{brief.product.audience}</dd>
            <dt className="label">Etapa</dt>
            <dd>
              <Badge>{brief.product.stage}</Badge>
            </dd>
          </dl>
          <div>
            <p className="label mb-2">Objetivos</p>
            {brief.objectives.length === 0 ? (
              <p className="text-sm text-muted-foreground">Sin objetivos.</p>
            ) : (
              <ul className="space-y-3">
                {brief.objectives.map((o) => (
                  <li key={o.id} className="flex gap-3 text-[15px]">
                    <span className="mono mt-0.5 shrink-0 text-xs text-muted-foreground">{o.id}</span>
                    <div className="min-w-0">
                      <p className="max-w-[68ch] leading-snug">{o.text}</p>
                      <p className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                        <span>Señal: {o.signal}</span>
                        <span aria-hidden>·</span>
                        <span className="tabular-nums">{o.due}</span>
                        {isPast(o.due) && <Badge tone="bad">vencido</Badge>}
                      </p>
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </div>
          <div className="grid gap-6 sm:grid-cols-2">
            <div>
              <p className="label mb-2">North stars</p>
              {brief.north_stars.length === 0 ? (
                <p className="text-sm text-muted-foreground">Ninguna.</p>
              ) : (
                <ul className="list-disc space-y-1 pl-5 text-[15px]">
                  {brief.north_stars.map((n) => (
                    <li key={n}>{n}</li>
                  ))}
                </ul>
              )}
            </div>
            <div>
              <p className="label mb-2">Fuera de alcance</p>
              {brief.out_of_scope.length === 0 ? (
                <p className="text-sm text-muted-foreground">Nada declarado.</p>
              ) : (
                <ul className="list-disc space-y-1 pl-5 text-[15px]">
                  {brief.out_of_scope.map((n) => (
                    <li key={n}>{n}</li>
                  ))}
                </ul>
              )}
            </div>
          </div>
        </div>
      )}
    </section>
  );
}

const REASON: Record<string, string> = {
  unresolved_source: "fuentes que no resuelven",
  invented_source: "fuentes inventadas",
  missing_objective: "prioridades sin objetivo válido",
  timeout: "timeouts",
  sin_datos: "“sin datos” repetido",
};

function Table({ head, children, label }: { head: string[]; children: React.ReactNode; label: string }) {
  return (
    <div role="region" aria-label={label} tabIndex={0} className="overflow-x-auto">
      <table className="w-full min-w-max text-left text-sm tabular-nums">
        <thead>
          <tr className="border-b border-border">
            {head.map((h, i) => (
              <th key={h} scope="col" className={`label py-2 pr-4 font-medium ${i ? "text-right" : ""}`}>
                {h}
              </th>
            ))}
          </tr>
        </thead>
        <tbody className="divide-y divide-border">{children}</tbody>
      </table>
    </div>
  );
}

const Cell = ({ children }: { children: React.ReactNode }) => <td className="py-2 pr-4 text-right">{children}</td>;
const AgentCell = ({ agent }: { agent: string }) => (
  <td className="py-2 pr-4">
    <RoleTag role={agent} />
  </td>
);

const rounds = (n: number | null): string => (n === null ? "—" : String(Math.round(n * 100) / 100));

export function AuditSection({ audit }: { audit: Loaded<AuditReport> }) {
  if (!audit.ok) {
    return (
      <section id="auditoria" className="mb-14 scroll-mt-20 md:scroll-mt-6">
        <SectionTitle>Auditoría</SectionTitle>
        <Failed what="la auditoría" error={audit.error} />
      </section>
    );
  }
  const r = audit.data;
  const m = r.meetings;
  const tiles = [
    { label: "Reuniones", value: String(m.count) },
    { label: "Check a la primera", value: percent(m.first_pass_rate) },
    { label: "Reparadas", value: percent(m.repair_rate) },
    { label: `Rondas de reparación, primeras ${r.repairs.baseline.minutes.length}`, value: rounds(r.repairs.baseline.per_meeting) },
    { label: `Rondas de reparación, después (${r.repairs.since.minutes.length})`, value: rounds(r.repairs.since.per_meeting) },
    { label: "Costo en síntesis", value: percent(m.synthesis_cost_share) },
    { label: "Fuentes inventadas", value: String(m.invented_sources) },
  ];
  const marks = Object.entries(r.marks);
  return (
    <section id="auditoria" className="mb-14 scroll-mt-20 md:scroll-mt-6">
      <SectionTitle aside={<span className="mono text-xs">{r.generated_at.slice(0, 10)}</span>}>Auditoría</SectionTitle>
      <div className="space-y-7 border-y border-border py-5">
        {r.ready_to_reflect.length > 0 ? (
          <div className="rounded-lg border border-status-waiting/40 bg-status-waiting/10 px-4 py-3">
            <p className="font-medium">Listos para reflexionar</p>
            <ul className="mt-1.5 space-y-1 text-sm">
              {r.ready_to_reflect.map((a) => (
                <li key={`${a.agent}-${a.failure_class}`} className="flex flex-wrap items-center gap-x-2">
                  <RoleTag role={a.agent} />
                  <span className="text-muted-foreground">
                    <span className="tabular-nums">{a.count}</span>{" "}
                    {a.reason === "rejected_priorities" ? "prioridades rechazadas" : (REASON[a.failure_class ?? ""] ?? a.failure_class)}
                  </span>
                </li>
              ))}
            </ul>
            <p className="mt-2 text-sm text-muted-foreground">
              Corré <span className="mono">/agentsi reflect</span> para curar la memory y proponer un cambio de prompt.
            </p>
          </div>
        ) : (
          <p className="text-sm text-muted-foreground">Listos para reflexionar: no hay evidencia suficiente.</p>
        )}

        <div>
          <p className="label mb-2">Configuración</p>
          {r.config.valid ? (
            <p className="text-sm">
              <Badge tone="ok">roster válido</Badge>
            </p>
          ) : (
            <ul role="alert" className="list-disc space-y-0.5 pl-5 text-sm text-status-neglected">
              {r.config.failures.map((f) => (
                <li key={f}>{f}</li>
              ))}
            </ul>
          )}
          {r.config.overdue_objectives.length > 0 && (
            <ul className="mt-2 space-y-0.5 text-sm text-status-waiting">
              {r.config.overdue_objectives.map((o) => (
                <li key={o.id}>
                  Objetivo vencido: <span className="mono text-xs">{o.id}</span> ({o.due}) {o.text}
                </li>
              ))}
            </ul>
          )}
        </div>

        <div>
          <p className="label mb-2">Reuniones (últimas {m.count})</p>
          <dl className="flex flex-wrap gap-x-10 gap-y-3">
            {tiles.map((t) => (
              <div key={t.label}>
                <dt className="label">{t.label}</dt>
                <dd className="mt-0.5 text-[17px] tabular-nums">{t.value}</dd>
              </div>
            ))}
          </dl>
          {m.agents.some((a) => a.meetings) && (
            <div className="mt-4">
              <Table
                label="Reuniones por agente"
                head={["Agente", "Respuestas", "Errores", "Timeouts", "Sin resolver", "Sin datos", "Costo medio"]}
              >
                {m.agents
                  .filter((a) => a.meetings)
                  .map((a) => (
                    <tr key={a.agent}>
                      <AgentCell agent={a.agent} />
                      <Cell>
                        {a.answers}/{a.meetings}
                      </Cell>
                      <Cell>{a.errors}</Cell>
                      <Cell>{a.timeouts}</Cell>
                      <Cell>{num(a.unresolved_sources_per_meeting)}</Cell>
                      <Cell>{num(a.sin_datos_per_meeting)}</Cell>
                      <Cell>{usd(a.mean_cost_usd)}</Cell>
                    </tr>
                  ))}
              </Table>
              <p className="mt-1 text-xs text-muted-foreground">Sin resolver y sin datos son promedios por reunión.</p>
            </div>
          )}
        </div>

        <div>
          <p className="label mb-2">Marcas por agente</p>
          {marks.length === 0 ? (
            <p className="text-sm text-muted-foreground">Sin prioridades.</p>
          ) : (
            <Table label="Marcas por agente" head={["Agente", "Aceptadas", "Con cambios", "Rechazadas", "Sin marcar"]}>
              {marks.map(([agent, c]) => (
                <tr key={agent}>
                  <AgentCell agent={agent} />
                  <Cell>{c.accepted}</Cell>
                  <Cell>{c.changed}</Cell>
                  <Cell>{c.rejected}</Cell>
                  <Cell>{c.unmarked}</Cell>
                </tr>
              ))}
            </Table>
          )}
        </div>

        <div>
          <p className="label mb-2">Memory</p>
          <Table label="Memory por agente" head={["Agente", "Activos", "Retirados", "Lecciones", "Nunca útiles", "Caracteres"]}>
            {r.memory.map((x) => (
              <tr key={x.agent}>
                <AgentCell agent={x.agent} />
                <Cell>{x.active}</Cell>
                <Cell>{x.retired}</Cell>
                <Cell>{x.lessons}</Cell>
                <Cell>{x.never_helpful}</Cell>
                <Cell>
                  {x.chars}/{x.max_chars}
                </Cell>
              </tr>
            ))}
          </Table>
        </div>

        <div>
          <p className="label mb-2">Reflexiones</p>
          <p className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
            <span>
              Última:{" "}
              {r.reflections.last ? (
                <span className="mono text-xs">{r.reflections.last.id}</span>
              ) : (
                <span className="text-muted-foreground">ninguna</span>
              )}
            </span>
            <span>
              <span className="tabular-nums">{r.reflections.pending_proposals}</span> propuestas pendientes
            </span>
            <span>
              <span className="tabular-nums">{r.reflections.reverted}</span> revertidas
            </span>
          </p>
        </div>
      </div>
    </section>
  );
}

const STATUS: Record<Proposal["status"], { label: string; tone: "ok" | "bad" | "warn" | "neutral" }> = {
  pending: { label: "pendiente", tone: "warn" },
  approved: { label: "aprobada", tone: "ok" },
  rejected: { label: "rechazada", tone: "bad" },
  discarded: { label: "descartada", tone: "neutral" },
  stale: { label: "obsoleta", tone: "warn" },
};

const METRIC: Record<string, string> = {
  first_pass_rate: "check a la primera",
  unresolved_per_case: "fuentes sin resolver",
  invented: "fuentes inventadas",
  missing_objectives: "prioridades sin objetivo",
  cost_usd: "costo",
};
const metrics = (keys: string[]) => keys.map((k) => METRIC[k] ?? k).join(", ");

const arrow = (a: string | number, b: string | number) => `${a} → ${b}`;

function ReplayTable({ replay }: { replay: ReplayReport }) {
  const t = replay.totals;
  return (
    <div className="space-y-2">
      {replay.aborted && <p className="text-sm text-status-neglected">{replay.aborted}</p>}
      {replay.rows.length > 0 && (
        <Table label="Replay" head={["Caso", "1ª pasada", "Sin resolver", "Inventadas", "Sin objetivo", "Costo"]}>
          {replay.rows.map((row) => (
            <tr key={row.minutes_id}>
              <td className="mono py-2 pr-4 text-xs">{row.minutes_id}</td>
              <Cell>{arrow(row.before.first_pass ? "sí" : "no", row.after.first_pass ? "sí" : "no")}</Cell>
              <Cell>{arrow(row.before.unresolved, row.after.unresolved)}</Cell>
              <Cell>{arrow(row.before.invented, row.after.invented)}</Cell>
              <Cell>{arrow(row.before.missing_objectives, row.after.missing_objectives)}</Cell>
              <Cell>{arrow(usd(row.before.cost_usd), usd(row.after.cost_usd))}</Cell>
            </tr>
          ))}
          {t && (
            <tr className="font-medium">
              <td className="py-2 pr-4">Total ({t.after.cases})</td>
              <Cell>{arrow(percent(t.before.first_pass_rate), percent(t.after.first_pass_rate))}</Cell>
              <Cell>{arrow(num(t.before.unresolved_per_case), num(t.after.unresolved_per_case))}</Cell>
              <Cell>{arrow(t.before.invented, t.after.invented)}</Cell>
              <Cell>{arrow(t.before.missing_objectives, t.after.missing_objectives)}</Cell>
              <Cell>{arrow(usd(t.before.cost_usd), usd(t.after.cost_usd))}</Cell>
            </tr>
          )}
        </Table>
      )}
      {t && (
        <p className="text-sm text-muted-foreground">
          {replay.verdict.passed ? "Mejora sin empeorar nada" : "No pasa el replay"}
          {replay.verdict.better.length > 0 && <> · mejora: {metrics(replay.verdict.better)}</>}
          {replay.verdict.worse.length > 0 && <> · empeora: {metrics(replay.verdict.worse)}</>}
          <span className="tabular-nums"> · costo del replay {usd(replay.cost_usd)} de {usd(replay.budget_usd)}</span>
        </p>
      )}
    </div>
  );
}

function ProposalBlock({ proposal }: { proposal: Proposal }) {
  const s = STATUS[proposal.status];
  return (
    <div className="space-y-3 border-t border-border px-5 py-4">
      <div className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3">
        <div className="min-w-0">
          <p className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span className="font-semibold">Propuesta de prompt</span>
            <RoleTag role={proposal.agent} />
            <Badge tone={s.tone}>{s.label}</Badge>
          </p>
          <p className="mono mt-1 break-all text-xs text-muted-foreground">{proposal.target}</p>
        </div>
        {proposal.status === "pending" && proposal.decision_id !== null && <DecideHint id={proposal.decision_id} />}
      </div>
      <p className="max-w-[68ch] text-[15px]">{proposal.rationale}</p>
      <p className="max-w-[68ch] text-sm text-muted-foreground">Efecto esperado: {proposal.expected_effect}</p>
      <ReplayTable replay={proposal.replay} />
      {proposal.diff.trim() && <DiffView diff={proposal.diff} label={`Diff de ${proposal.target}`} />}
    </div>
  );
}

function ReflectionCard({ entry, product }: { entry: ReflectionEntry; product: string }) {
  const reverted = entry.status === "reverted";
  const changes = entry.memory.filter((c) => c.diff.trim());
  return (
    <article className="rounded-lg border border-border bg-card">
      <header className="flex flex-wrap items-start justify-between gap-x-4 gap-y-3 px-5 py-4">
        <div className="min-w-0">
          <p className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span className="font-semibold">{dateOf({ id: entry.id, created_at: entry.created_at })}</span>
            <span className="mono text-xs text-muted-foreground">{entry.id}</span>
            {entry.reverts ? <Badge tone="warn">revierte {entry.reverts}</Badge> : <Badge>manual</Badge>}
            {reverted && <Badge tone="warn">revertida</Badge>}
          </p>
          {entry.cost_usd ? <p className="mt-1 text-xs tabular-nums text-muted-foreground">Costo {usd(entry.cost_usd)}</p> : null}
        </div>
        {canRevert(entry) && <CliHint>{`agentsi reflect revert ${entry.id} --product ${product}`}</CliHint>}
      </header>

      <div className="space-y-4 border-t border-border px-5 py-4">
        <details>
          <summary className="inline-flex items-center gap-2 text-sm text-muted-foreground">
            <ChevronRight className="chev size-4 text-muted-foreground transition-transform" />
            <span className="tabular-nums">{entry.signals.length}</span> señales
          </summary>
          {entry.signals.length > 0 && (
            <ul className="mt-3 space-y-2 text-sm">
              {entry.signals.map((sig, i) => (
                <li key={i} className="flex flex-wrap items-baseline gap-x-2 gap-y-0.5">
                  <RoleTag role={sig.agent} />
                  <Badge>{sig.kind}</Badge>
                  <span className="min-w-0 break-words text-muted-foreground">{sig.detail}</span>
                  <span className="mono text-xs text-muted-foreground">{sig.ref}</span>
                </li>
              ))}
            </ul>
          )}
        </details>

        {changes.length === 0 ? (
          <p className="text-sm text-muted-foreground">Sin cambios de memory.</p>
        ) : (
          changes.map((c) => (
            <div key={c.agent} className="space-y-1.5">
              <p className="flex flex-wrap items-center gap-x-3 text-sm">
                <RoleTag role={c.agent} />
                <span className="text-xs text-muted-foreground">
                  memory/{c.agent}.json · <span className="tabular-nums">{c.added.length}</span> agregados ·{" "}
                  <span className="tabular-nums">{c.retired.length}</span> retirados
                  {!!c.restored?.length && (
                    <>
                      {" "}· <span className="tabular-nums">{c.restored.length}</span> restaurados
                    </>
                  )}
                </span>
              </p>
              <DiffView diff={c.diff} label={`Cambios de memory de ${c.agent}`} />
            </div>
          ))
        )}
      </div>

      {entry.proposal && <ProposalBlock proposal={entry.proposal} />}
    </article>
  );
}

export function ReflectionsSection({ reflections, product }: { reflections: ReflectionEntry[]; product: string }) {
  return (
    <section id="reflexiones" className="scroll-mt-20 md:scroll-mt-6">
      <SectionTitle aside={reflections.length || null}>Reflexiones</SectionTitle>
      {reflections.length === 0 ? (
        <Empty>
          Todavía no hubo reflexiones. Cuando la auditoría marque agentes listos, corré{" "}
          <span className="mono">/agentsi reflect</span>.
        </Empty>
      ) : (
        <div className="space-y-5">
          {reflections.map((e) => (
            <ReflectionCard key={e.id} entry={e} product={product} />
          ))}
        </div>
      )}
    </section>
  );
}
