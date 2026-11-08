import Link from "next/link";
import { notFound } from "next/navigation";
import { AlertTriangle, ArchiveX, CircleCheckBig, CircleDashed, type LucideIcon } from "lucide-react";
import { Crumbs, Empty, GlyphHeading } from "@/components/bits";
import { CliHint } from "@/components/cli-hint";
import { RoleTag } from "@/components/roster";
import { StoryBadges } from "@/components/stories/story-badges";
import { Pager } from "@/components/pager";
import { paginate } from "@/lib/paginate";
import {
  STORIES_PAGE_PARAM,
  STORIES_SIZES,
  STORIES_SIZE_PARAM,
  parseStoriesSize,
  storiesSizeHref,
} from "@/lib/stories-page-size";
import { findProduct } from "@/lib/registry";
import { groupStories, readStories, type StoryStatus } from "@/lib/stories";

export const dynamic = "force-dynamic";

const ICON: Record<StoryStatus, LucideIcon> = {
  not_implemented: CircleDashed,
  implemented: CircleCheckBig,
  discarded: ArchiveX,
};

const EMPTY: Record<StoryStatus, string> = {
  not_implemented: "No hay stories abiertas.",
  implemented: "Todavía no se implementó ninguna story.",
  discarded: "No descartaste ninguna story.",
};

export default async function StoriesPage({
  params,
  searchParams,
}: {
  params: Promise<{ product: string }>;
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const { product: pid } = await params;
  const query = await searchParams;
  const product = findProduct(pid);
  if (!product) notFound();

  const crumbs = (
    <Crumbs items={[{ label: "Productos", href: "/" }, { label: product.name, href: `/p/${pid}` }, { label: "Stories" }]} />
  );

  if (!product.stories) {
    return (
      <div>
        {crumbs}
        <h1 className="mb-6 text-3xl font-semibold tracking-[-0.02em]">Stories</h1>
        <Empty>
          Stories mode está apagado para {product.name}. Agregá un bloque <span className="mono">stories</span> a este producto en{" "}
          <span className="mono">agentsi.config.json</span>.
        </Empty>
      </div>
    );
  }

  const { stories, error } = await readStories(pid);
  const allGroups = groupStories(stories);
  const size = parseStoriesSize(query[STORIES_SIZE_PARAM]);
  const paged = paginate(
    allGroups.flatMap((g) => g.stories),
    query[STORIES_PAGE_PARAM],
    size,
  );
  const onPage = new Set(paged.items.map((s) => s.id));
  const groups = allGroups
    .map((g) => ({ ...g, total: g.stories.length, stories: g.stories.filter((s) => onPage.has(s.id)) }))
    .filter((g) => g.stories.length > 0 || paged.total === 0);
  const path = `/p/${pid}/stories`;

  return (
    <div>
      {crumbs}
      <div className="mb-10 flex flex-wrap items-end justify-between gap-4">
        <div>
          <h1 className="text-3xl font-semibold tracking-[-0.02em]">Stories</h1>
          <p className="mt-2 max-w-xl text-sm text-muted-foreground">
            Cada story dice qué quiere un usuario y cómo se prueba. Aprobada, una sesión escribe los tests y el código en una rama.
          </p>
        </div>
        <div className="flex flex-col items-start gap-1.5 sm:items-end">
          <CliHint>{`agentsi story new "…" --product ${pid}`}</CliHint>
          <CliHint>{`agentsi story suggest --product ${pid}`}</CliHint>
        </div>
      </div>

      {error && (
        <section role="alert" className="mb-8 flex gap-3 rounded-lg border border-destructive/40 bg-destructive/5 px-5 py-4">
          <AlertTriangle className="mt-0.5 size-5 shrink-0 text-destructive" aria-hidden />
          <div className="text-sm">
            <p className="font-medium">No pude leer las stories</p>
            <p className="mt-1 text-muted-foreground">{error}</p>
          </div>
        </section>
      )}

      <div className="space-y-12">
        {groups.map((g) => (
          <section key={g.status} id={g.status} className="scroll-mt-20 md:scroll-mt-6">
            <GlyphHeading Icon={ICON[g.status]} count={g.total}>
              {g.label}
            </GlyphHeading>
            {g.stories.length === 0 ? (
              <Empty>{EMPTY[g.status]}</Empty>
            ) : (
              <ul className="flex flex-col gap-2.5">
                {g.stories.map((s) => (
                  <li key={s.id}>
                    <Link
                      href={`/p/${pid}/stories/${s.id}`}
                      className="group block rounded-lg bg-card px-5 py-4 shadow-soft transition-colors hover:bg-muted/60"
                    >
                      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
                        <span className="mono text-xs text-muted-foreground">{s.id}</span>
                        <StoryBadges story={s} status={false} />
                      </div>
                      <p className="mt-2 text-balance text-[16px] font-semibold leading-snug underline-offset-4 group-hover:underline">
                        {s.title}
                      </p>
                      <p className="mt-1 line-clamp-1 text-sm text-muted-foreground">
                        {s.as_a ? `Como ${s.as_a}, quiero ${s.i_want}` : `Quiero ${s.i_want}`}
                      </p>
                      <div className="mt-2.5 flex flex-wrap items-center gap-x-4 gap-y-1 text-[13px] text-muted-foreground">
                        <RoleTag role={s.owner_role} />
                        <span>
                          <span className="mono">{s.criteria.length}</span> criterios
                        </span>
                      </div>
                    </Link>
                  </li>
                ))}
              </ul>
            )}
          </section>
        ))}
      </div>

      {paged.total > 0 && (
        <div className="mt-8">
          <div className="flex items-center justify-end gap-1 text-[13px] text-muted-foreground">
            <span className="mr-1">Por página</span>
            {STORIES_SIZES.map((n) => (
              <Link
                key={n}
                href={storiesSizeHref(path, query, n)}
                aria-current={n === size ? "true" : undefined}
                className={`rounded-md px-2 py-1 tabular-nums hover:text-foreground ${n === size ? "bg-card text-foreground" : ""}`}
              >
                {n}
              </Link>
            ))}
          </div>
          <Pager page={paged} param={STORIES_PAGE_PARAM} path={path} params={query} anchor="stories" label="stories" numbered />
        </div>
      )}
    </div>
  );
}
