import Link from "next/link";
import { notFound } from "next/navigation";
import { BookOpen, Users } from "lucide-react";
import { Crumbs, Empty, MarksSummaryInline, SectionTitle, StatusBadge } from "@/components/bits";
import { DecideHint } from "@/components/cli-hint";
import { RoleDot } from "@/components/roster";
import { Pager } from "@/components/pager";
import { HealthChip } from "@/components/health-chip";
import { buttonVariants } from "@/components/ui/button";
import { decisionLink } from "@/lib/decisions";
import { compact, dateOf, usd } from "@/lib/format";
import { paginate } from "@/lib/paginate";
import {
  DECISIONS_SIZES,
  DECISIONS_SIZE_PARAM,
  decisionsSizeHref,
  parseDecisionsSize,
} from "@/lib/decisions-page-size";
import { findProduct } from "@/lib/registry";
import { roleMeta, sortRoles } from "@/lib/roles";
import { loadInbox } from "@/lib/inbox-data";
import { listMinutes, pendingDecisions, productUsage, summarizeMarks } from "@/lib/workspace";

export const dynamic = "force-dynamic";

/** Section id and query param of the pending decisions; both are existing URLs. */
const DECISIONS_ANCHOR = "decisiones";

export default async function ProductPage({
  params,
  searchParams,
}: {
  params: Promise<{ product: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const [{ product: pid }, query] = await Promise.all([params, searchParams]);
  const product = findProduct(pid);
  if (!product) notFound();

  const [allMinutes, allDecisions, usage, inbox] = await Promise.all([
    listMinutes(pid),
    pendingDecisions(pid),
    productUsage(pid),
    loadInbox(),
  ]);
  const state = inbox.products.find((p) => p.id === pid);
  const heardIds = new Set(inbox.minutes.filter((m) => m.product === pid && m.heard).map((m) => m.id));

  const minutes = paginate(allMinutes, query.minutes, 10);
  const decisionsSize = parseDecisionsSize(query[DECISIONS_SIZE_PARAM]);
  const decisions = paginate(allDecisions, query[DECISIONS_ANCHOR], decisionsSize);
  const path = `/p/${pid}`;

  const tiles = [
    { label: "Tokens in", value: compact(usage.input) },
    { label: "Tokens out", value: compact(usage.output) },
    { label: "Cache read", value: compact(usage.cacheRead) },
    { label: "Cache write", value: compact(usage.cacheWrite) },
    { label: "Costo", value: usd(usage.cost) },
  ];

  return (
    <div>
      <Crumbs items={[{ label: "Productos", href: "/" }, { label: product.name }]} />

      <div className="mb-10 flex flex-wrap items-end justify-between gap-4">
        <div>
          <div className="flex flex-wrap items-center gap-3">
            <h1 className="text-3xl font-semibold tracking-[-0.02em]">{product.name}</h1>
            <span className="mono text-sm text-muted-foreground">{product.id}</span>
            {state && <HealthChip health={state.health} days={state.daysSinceMeeting} />}
          </div>
          <div className="mt-2 flex items-center gap-4 text-sm text-muted-foreground">
            <span className="flex items-center gap-3">
              {sortRoles(product.roles).map((r) => (
                <span key={r} className="flex items-center gap-1.5">
                  <RoleDot role={r} />
                  {roleMeta(r).label}
                </span>
              ))}
            </span>
          </div>
        </div>
        <div className="flex flex-wrap gap-2">
          <Link href={`/p/${pid}/memory`} className={buttonVariants({ variant: "outline" })}>
            <BookOpen className="size-4" /> Memory de los roles
          </Link>
          <Link href={`/p/${pid}/roles`} className={buttonVariants({ variant: "outline" })}>
            <Users className="size-4" /> Roles
          </Link>
        </div>
      </div>

      <section id={DECISIONS_ANCHOR} className="mb-14 scroll-mt-20 md:scroll-mt-6">
        <SectionTitle
          aside={decisions.total || null}
        >
          Decisiones pendientes
        </SectionTitle>
        {decisions.total === 0 ? (
          <Empty>Nada espera tu aprobación.</Empty>
        ) : (
          <ul className="divide-y divide-border border-y border-border">
            {decisions.items.map((d) => {
              const link = decisionLink(pid, d);
              return (
                <li key={d.id} className="flex flex-wrap items-start justify-between gap-4 py-4">
                  <div className="min-w-0 max-w-2xl">
                    <p className="font-medium">{d.text}</p>
                    {d.why && <p className="mt-1 text-sm text-muted-foreground">{d.why}</p>}
                    {d.action && <p className="mt-1 text-sm text-muted-foreground">Acción: {d.action}</p>}
                    {link && (
                      <Link
                        href={link.href}
                        className="mono mt-2 inline-block text-xs text-muted-foreground underline-offset-4 hover:text-foreground hover:underline"
                      >
                        {link.label}
                      </Link>
                    )}
                  </div>
                  <DecideHint id={d.id} />
                </li>
              );
            })}
          </ul>
        )}
        {decisions.total > 0 && (
          <div className="mt-3 flex items-center gap-1 text-[13px] text-muted-foreground">
            <span className="mr-1">Por página:</span>
            {DECISIONS_SIZES.map((n) => (
              <Link
                key={n}
                href={decisionsSizeHref(path, query, DECISIONS_ANCHOR, n, DECISIONS_ANCHOR)}
                aria-current={n === decisionsSize ? "true" : undefined}
                className={`rounded-md px-2 py-1 tabular-nums hover:text-foreground ${n === decisionsSize ? "bg-card text-foreground" : ""}`}
              >
                {n}
              </Link>
            ))}
          </div>
        )}
        <Pager page={decisions} param={DECISIONS_ANCHOR} path={path} params={query} anchor={DECISIONS_ANCHOR} label="decisiones pendientes" />
      </section>

      <section className="mb-14">
        <SectionTitle>Uso acumulado</SectionTitle>
        {usage.rows === 0 ? (
          <Empty>Todavía no hay usage registrado.</Empty>
        ) : (
          <dl className="flex flex-wrap gap-x-10 gap-y-3 border-y border-border py-4">
            {tiles.map((t) => (
              <div key={t.label}>
                <dt className="label">{t.label}</dt>
                <dd className="mt-0.5 text-[17px] tabular-nums">{t.value}</dd>
              </div>
            ))}
          </dl>
        )}
      </section>

      <section id="minutes" className="scroll-mt-20 md:scroll-mt-6">
        <SectionTitle aside={minutes.total}>Minutes</SectionTitle>
        {minutes.total === 0 ? (
          <Empty>
            No hay minutes para este producto. Convocá una reunión con{" "}
            <span className="mono">agentsi meet --product {pid} --question &quot;…&quot;</span>
          </Empty>
        ) : (
          <ul className="divide-y divide-border border-y border-border">
            {minutes.items.map((m) => (
              <li key={m.id}>
                <Link
                  href={`/p/${pid}/m/${m.id}`}
                  className="group -mx-3 block rounded-md px-3 py-4 transition-colors hover:bg-card"
                >
                  <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
                    <span className="mono text-xs text-muted-foreground">{m.id}</span>
                    <StatusBadge status={m.status} />
                    <span className="text-xs text-muted-foreground">{dateOf(m)}</span>
                    {heardIds.has(m.id) && <span className="text-xs text-status-fresh">escuchada</span>}
                  </div>
                  <p className="mt-1.5 text-balance text-[16px] font-medium leading-snug underline-offset-4 group-hover:underline">{m.question}</p>
                  <div className="mt-2">
                    <MarksSummaryInline s={summarizeMarks(m.priorities)} total={m.priorities?.length ?? 0} />
                  </div>
                </Link>
              </li>
            ))}
          </ul>
        )}
        <Pager page={minutes} param="minutes" path={path} params={query} anchor="minutes" label="minutes" />
      </section>
    </div>
  );
}
