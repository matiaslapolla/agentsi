import Link from "next/link";
import { notFound } from "next/navigation";
import { AlertTriangle, CheckCircle2, Headphones } from "lucide-react";
import { ArchiveButton } from "@/components/archive-button";
import { Crumbs, Empty, StatusBadge } from "@/components/bits";
import { CliHint, DecideHint, MarkBadge, MarkState } from "@/components/cli-hint";
import { MeetingProgress } from "@/components/inbox/meeting-progress";
import { FoldList } from "@/components/minutes/fold-list";
import { MinutesTabs, type MinutesTabItem } from "@/components/minutes/minutes-tabs";
import { SpeechBubble } from "@/components/minutes/speech-bubble";
import { SummaryTile } from "@/components/minutes/summary-tile";
import { RoleAvatar, RoleDot, RoleTag } from "@/components/roster";
import { SourceChip } from "@/components/source-chip";
import { AccordionContent, AccordionItem, AccordionTrigger } from "@/components/ui/accordion";
import { Badge } from "@/components/ui/badge";
import { decisionStatus, withRows } from "@/lib/decisions";
import { clock, compact, dateOf, usd } from "@/lib/format";
import { meetingProgress, questBoard, questHref, type Quest } from "@/lib/inbox";
import { loadInbox } from "@/lib/inbox-data";
import { findProduct } from "@/lib/registry";
import { roleMeta, sortRoles } from "@/lib/roles";
import { DECISIONS_TAB, USAGE_TAB } from "@/lib/minutes-tabs";
import { audioFile, decisionsFor, getMinutes, getTimings, getUsage, sumUsage } from "@/lib/workspace";

export const dynamic = "force-dynamic";

function nextQuest(q: Quest | undefined, names: Map<string, string>) {
  if (!q) return null;
  const name = names.get(q.product) ?? q.product;
  const label =
    q.kind === "listen" ? `escuchar la llamada de ${name}` : q.kind === "meet" ? `una reunión en ${name}` : `cerrar una reunión de ${name}`;
  return { label, href: questHref(q) };
}

function firstLine(text: string | undefined) {
  return (text ?? "").split("\n").find((l) => l.trim())?.trim() ?? "Sin respuesta.";
}

export default async function MinutesPage({ params }: { params: Promise<{ product: string; id: string }> }) {
  const { product: pid, id } = await params;
  const product = findProduct(pid);
  const minutes = product ? await getMinutes(pid, id) : null;
  if (!product || !minutes) notFound();

  const usage = await getUsage(pid, id);
  const totals = sumUsage(usage);
  const timings = getTimings(pid, id);
  const hasAudio = !!audioFile(pid, id);
  const duration = timings.length ? Math.max(...timings.map((t) => t.end)) : 0;
  const [dbDecisions, inbox] = await Promise.all([decisionsFor(id), loadInbox()]);
  const names = new Map(inbox.products.map((p) => [p.id, p.name]));
  const next = nextQuest(questBoard(inbox.products).find((q) => q.minutesId !== id), names);
  const roleIds = sortRoles(Object.keys(minutes.roles ?? {}));
  const check = minutes.check;
  const priorities = minutes.priorities ?? [];
  const disagreements = minutes.disagreements ?? [];
  const decisions = withRows(minutes.decisions ?? [], dbDecisions);

  const progress = meetingProgress({
    priorities: priorities.map((p) => ({ mark: p.mark ?? null })),
    decisions: decisions.map((d) => ({ status: decisionStatus(d) })),
  });
  const marks = priorities.map((p) => !!p.mark);
  const decidedFlags = decisions.map((d) => decisionStatus(d) !== "pending");

  const tabs: MinutesTabItem[] = [
    {
      value: "priorities",
      label: "Priorities",
      count: priorities.length,
      content: !priorities.length ? (
        <Empty>Estas minutes no tienen priorities.</Empty>
      ) : (
        <div className="space-y-3">
          {priorities.some((p) => !p.mark) && (
            <div className="flex justify-end">
              <CliHint>agentsi mark</CliHint>
            </div>
          )}
          <FoldList prefix="p" className="flex flex-col gap-2.5">
            {priorities.map((p, i) => (
              <AccordionItem key={i} value={`p-${i}`} id={`p-${i}`} className="scroll-mt-32 rounded-lg border-none bg-card px-4 shadow-soft sm:px-5">
                <AccordionTrigger className="min-w-0 items-center gap-3 py-4 hover:no-underline">
                  <span className="flex min-w-0 flex-1 items-start gap-3">
                    <span className={`mt-0.5 w-5 shrink-0 text-right text-[15px] tabular-nums ${p.mark ? "text-status-fresh" : "text-muted-foreground"}`}>
                      {p.mark ? <CheckCircle2 className="ml-auto mt-0.5 size-4" aria-label="marcada" /> : i + 1}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block text-pretty text-[16px] font-semibold leading-snug">{p.title}</span>
                      <span className="mt-1.5 flex flex-wrap items-center gap-2 font-normal">
                        {p.owner_role && <RoleTag role={p.owner_role} />}
                        <MarkBadge mark={p.mark ?? null} />
                      </span>
                    </span>
                  </span>
                </AccordionTrigger>
                <AccordionContent className="pb-5 pl-8 text-[15px]">
                  {p.objective && (
                    <Badge tone="accent" title="Objetivo del brief" className="mb-2">
                      {p.objective}
                    </Badge>
                  )}
                  <p className="max-w-[68ch] text-muted-foreground">{p.why}</p>
                  {!!p.sources?.length && (
                    <div className="mt-3 flex flex-wrap gap-1.5">
                      {p.sources.map((s, j) => (
                        <SourceChip key={j} source={s} product={pid} />
                      ))}
                    </div>
                  )}
                  <div className="mt-4">
                    <MarkState minutesId={minutes.id} idx={i} mark={p.mark ?? null} isNew={!!p.is_new} />
                  </div>
                </AccordionContent>
              </AccordionItem>
            ))}
          </FoldList>
        </div>
      ),
    },
    ...(disagreements.length
      ? [
          {
            value: "disagreements" as const,
            label: "Disagreements",
            count: disagreements.length,
            content: (
              <div className="flex flex-col gap-10">
                {disagreements.map((d, i) => (
                  <div key={i}>
                    <h3 className="text-[17px] font-semibold">{d.topic}</h3>
                    <div className="mt-4 grid gap-x-5 gap-y-6 md:grid-cols-2">
                      {d.positions.map((pos, j) => (
                        <SpeechBubble key={j} role={pos.role} tone="white">
                          {pos.view}
                        </SpeechBubble>
                      ))}
                    </div>
                  </div>
                ))}
              </div>
            ),
          },
        ]
      : []),
    {
      value: DECISIONS_TAB,
      label: "Decisiones",
      count: decisions.length,
      content: !decisions.length ? (
        <Empty>Estas minutes no proponen decisiones.</Empty>
      ) : (
        <ul className="flex flex-col gap-2.5">
          {decisions.map(({ row, ...d }, i) => (
            <li
              key={i}
              id={`d-${i}`}
              className="flex scroll-mt-32 flex-wrap items-start justify-between gap-4 rounded-lg bg-card px-5 py-4 shadow-soft"
            >
              <div className="max-w-2xl">
                <p className="font-medium">{d.text}</p>
                {d.why && <p className="mt-1 text-sm text-muted-foreground">{d.why}</p>}
                {d.action && <p className="mt-1 text-sm text-muted-foreground">Acción: {d.action}</p>}
              </div>
              {row?.status === "pending" ? (
                <DecideHint id={row.id} />
              ) : row ? (
                <Badge tone={row.status === "approved" ? "ok" : "bad"}>{row.status === "approved" ? "aprobada" : "rechazada"}</Badge>
              ) : (
                <Badge>sin registrar</Badge>
              )}
            </li>
          ))}
        </ul>
      ),
    },
    ...(roleIds.length
      ? [
          {
            value: "roles" as const,
            label: "Respuesta de cada rol",
            count: roleIds.length,
            content: (
              <FoldList className="flex flex-col gap-2.5">
                {roleIds.map((r) => {
                  const a = minutes.roles![r];
                  return (
                    <AccordionItem key={r} value={r} className="rounded-lg border-none bg-card px-4 shadow-soft sm:px-5">
                      <AccordionTrigger className="min-w-0 items-center py-3 hover:no-underline">
                        <span className="flex min-w-0 flex-1 items-center gap-3">
                          <RoleAvatar role={r} size={28} />
                          <span className="min-w-0 flex-1">
                            <span className="flex flex-wrap items-baseline gap-x-2">
                              <span className="text-[15px] font-semibold">{roleMeta(r).label}</span>
                              {a.model && <span className="mono text-xs font-normal text-muted-foreground">{a.model}</span>}
                            </span>
                            <span className="block truncate text-sm font-normal text-muted-foreground">{firstLine(a.answer)}</span>
                          </span>
                        </span>
                      </AccordionTrigger>
                      <AccordionContent className="space-y-5 pb-5 text-[15px]">
                        <SpeechBubble role={r}>
                          <span className="whitespace-pre-line">{a.answer || "Sin respuesta."}</span>
                        </SpeechBubble>
                        {!!a.concerns?.length && (
                          <div>
                            <p className="label mb-1">Concerns</p>
                            <ul className="list-disc space-y-0.5 pl-5 text-muted-foreground">
                              {a.concerns.map((c, i) => (
                                <li key={i}>{c}</li>
                              ))}
                            </ul>
                          </div>
                        )}
                        {!!a.memory_notes?.length && (
                          <div>
                            <p className="label mb-1">Notas para su memory</p>
                            <ul className="list-disc space-y-0.5 pl-5 text-muted-foreground">
                              {a.memory_notes.map((c, i) => (
                                <li key={i}>{c}</li>
                              ))}
                            </ul>
                          </div>
                        )}
                      </AccordionContent>
                    </AccordionItem>
                  );
                })}
              </FoldList>
            ),
          },
        ]
      : []),
    {
      value: USAGE_TAB,
      label: "Uso de esta reunión",
      count: usage.length,
      content:
        usage.length === 0 ? (
          <Empty>No hay usage registrado para estas minutes.</Empty>
        ) : (
          <div className="overflow-x-auto rounded-lg bg-card px-4 shadow-soft">
            <table className="w-full min-w-[640px] text-sm">
              <thead>
                <tr className="label border-b border-border text-left">
                  <th className="py-2.5 pr-3 font-medium">Rol</th>
                  <th className="px-3 py-2.5 font-medium">Modelo</th>
                  <th className="px-3 py-2.5 text-right font-medium">In</th>
                  <th className="px-3 py-2.5 text-right font-medium">Out</th>
                  <th className="px-3 py-2.5 text-right font-medium">Cache read</th>
                  <th className="px-3 py-2.5 text-right font-medium">Cache write</th>
                  <th className="py-2.5 pl-3 text-right font-medium">Costo</th>
                </tr>
              </thead>
              <tbody className="mono tabular-nums">
                {usage.map((u, i) => (
                  <tr key={i} className="border-b border-border">
                    <td className="py-2.5 pr-3 font-sans">
                      <span className="flex items-center gap-2">
                        <RoleDot role={u.role} />
                        {roleMeta(u.role).label}
                      </span>
                    </td>
                    <td className="px-3 py-2.5 text-muted-foreground">{u.model}</td>
                    <td className="px-3 py-2.5 text-right">{compact(u.input_tokens)}</td>
                    <td className="px-3 py-2.5 text-right">{compact(u.output_tokens)}</td>
                    <td className="px-3 py-2.5 text-right">{compact(u.cache_read)}</td>
                    <td className="px-3 py-2.5 text-right">{compact(u.cache_write)}</td>
                    <td className="py-2.5 pl-3 text-right">{usd(Number(u.cost_usd) || 0)}</td>
                  </tr>
                ))}
              </tbody>
              <tfoot className="mono tabular-nums">
                <tr>
                  <td className="py-3 pr-3 font-sans font-semibold" colSpan={2}>
                    Total
                  </td>
                  <td className="px-3 py-3 text-right">{compact(totals.input)}</td>
                  <td className="px-3 py-3 text-right">{compact(totals.output)}</td>
                  <td className="px-3 py-3 text-right">{compact(totals.cacheRead)}</td>
                  <td className="px-3 py-3 text-right">{compact(totals.cacheWrite)}</td>
                  <td className="py-3 pl-3 text-right font-semibold">{usd(totals.cost)}</td>
                </tr>
              </tfoot>
            </table>
          </div>
        ),
    },
  ];

  return (
    <div>
      <Crumbs
        items={[
          { label: "Productos", href: "/" },
          { label: product.name, href: `/p/${pid}` },
          { label: "Minutes" },
        ]}
      />

      <ArchiveButton product={pid} minutesId={id} archived={!!minutes.archived} />

      <header className="mb-8">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
          <span className="mono text-xs text-muted-foreground">{minutes.id}</span>
          <StatusBadge status={minutes.status} />
          <span className="text-xs text-muted-foreground">{dateOf(minutes)}</span>
          {minutes.scope && <Badge>{minutes.scope}</Badge>}
          {check?.passed && (
            <span className="inline-flex items-center gap-1 text-xs text-muted-foreground">
              <CheckCircle2 className="size-3.5 text-status-fresh" aria-hidden />
              Pasó la verificación{check.repaired && " · reparada una vez por el PM"}
            </span>
          )}
        </div>
        <h1 className="mt-3 max-w-3xl text-balance text-[28px] font-semibold leading-tight tracking-[-0.02em]">{minutes.question}</h1>
      </header>

      {check && !check.passed && (
        <section role="alert" className="mb-8 flex gap-3 rounded-lg border border-destructive/40 bg-destructive/5 px-5 py-4">
          <AlertTriangle className="mt-0.5 size-5 shrink-0 text-destructive" aria-hidden />
          <div className="text-sm">
            <p className="font-medium">
              No pasó la verificación
              {check.repaired && <span className="font-normal text-muted-foreground"> · reparada una vez por el PM</span>}
            </p>
            {!!check.failures?.length && (
              <ul className="mt-1.5 list-disc space-y-0.5 pl-5 text-muted-foreground">
                {check.failures.map((f, i) => (
                  <li key={i}>{f}</li>
                ))}
              </ul>
            )}
          </div>
        </section>
      )}

      {marks.length + decidedFlags.length > 0 && (
        <MeetingProgress progress={progress} marks={marks} decisions={decidedFlags} next={next} />
      )}

      <section aria-label="Resumen" className="mb-10 flex flex-col gap-3">
        {(minutes.script?.length || hasAudio) && (
          <Link
            href={`/p/${pid}/m/${id}/call`}
            className="group flex items-center justify-between gap-4 rounded-lg bg-card px-4 py-3.5 shadow-soft transition-colors hover:bg-muted"
          >
            <span className="flex items-center gap-4">
              <span className="grid size-10 shrink-0 place-items-center rounded-full bg-primary text-primary-foreground">
                <Headphones className="size-[18px]" aria-hidden />
              </span>
              <span>
                <span className="block font-semibold">Escuchar la llamada</span>
                <span className="block text-sm text-muted-foreground">
                  {hasAudio
                    ? `${minutes.script?.length ?? timings.length} segmentos${duration ? ` · ${clock(duration)}` : ""}`
                    : "Audio todavía sin renderizar"}
                </span>
              </span>
            </span>
            <span className="hidden gap-1.5 sm:flex">
              {sortRoles(Array.from(new Set((minutes.script ?? []).map((s) => s.role)))).map((r) => (
                <RoleAvatar key={r} role={r} size={28} />
              ))}
            </span>
          </Link>
        )}
        <div className="grid grid-cols-2 gap-3 md:grid-cols-4">
          <SummaryTile label="Priorities marcadas" value={marks.filter(Boolean).length} total={marks.length} />
          <SummaryTile label="Decisiones decididas" value={decidedFlags.filter(Boolean).length} total={decidedFlags.length} />
          <SummaryTile label="Disagreements" value={disagreements.length} />
          <SummaryTile label="Costo" value={usage.length ? usd(totals.cost) : "—"} />
        </div>
      </section>

      <MinutesTabs tabs={tabs} />
    </div>
  );
}
