import Link from "next/link";
import { ChevronLeft, ChevronRight } from "lucide-react";
import { cn } from "@/lib/cn";
import type { Page } from "@/lib/paginate";

type Params = Record<string, string | string[] | undefined>;

function hrefFor(path: string, params: Params, param: string, page: number, anchor: string): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (k === param || v === undefined) continue;
    for (const one of Array.isArray(v) ? v : [v]) q.append(k, one);
  }
  if (page > 1) q.set(param, String(page));
  const qs = q.toString();
  return `${path}${qs ? `?${qs}` : ""}#${anchor}`;
}

const step = "inline-flex h-10 items-center gap-1 rounded-md px-2.5 text-sm transition-colors sm:h-8";

/** Prev/next links for one list; `anchor` is the list's section id, so the page lands back on it. */
export function Pager({
  page,
  param,
  path,
  params,
  anchor,
  label,
  numbered = false,
}: {
  page: Page<unknown>;
  param: string;
  path: string;
  params: Params;
  anchor: string;
  label: string;
  numbered?: boolean;
}) {
  if (page.pages <= 1) return null;
  const prev = page.page > 1 ? hrefFor(path, params, param, page.page - 1, anchor) : null;
  const next = page.page < page.pages ? hrefFor(path, params, param, page.page + 1, anchor) : null;
  return (
    <nav aria-label={`Páginas de ${label}`} className="mt-3 flex items-center justify-between gap-4">
      <p className="text-[13px] tabular-nums text-muted-foreground">
        {page.from}–{page.to} de {page.total}
      </p>
      <div className="flex items-center gap-1">
        {prev ? (
          <Link href={prev} className={cn(step, "text-muted-foreground hover:bg-card hover:text-foreground")} rel="prev">
            <ChevronLeft className="size-4" aria-hidden /> Anterior
          </Link>
        ) : (
          <span className={cn(step, "text-muted-foreground opacity-50")} aria-disabled="true">
            <ChevronLeft className="size-4" aria-hidden /> Anterior
          </span>
        )}
        {numbered ? (
          Array.from({ length: page.pages }, (_, i) => i + 1).map((n) =>
            n === page.page ? (
              <span key={n} className="px-1.5 text-[13px] tabular-nums text-foreground" aria-current="page">
                {n}
              </span>
            ) : (
              <Link
                key={n}
                href={hrefFor(path, params, param, n, anchor)}
                className={cn(step, "tabular-nums text-muted-foreground hover:bg-card hover:text-foreground")}
              >
                {n}
              </Link>
            ),
          )
        ) : (
          <span className="px-1 text-[13px] tabular-nums text-muted-foreground" aria-current="page">
            {page.page}/{page.pages}
          </span>
        )}
        {next ? (
          <Link href={next} className={cn(step, "text-muted-foreground hover:bg-card hover:text-foreground")} rel="next">
            Siguiente <ChevronRight className="size-4" aria-hidden />
          </Link>
        ) : (
          <span className={cn(step, "text-muted-foreground opacity-50")} aria-disabled="true">
            Siguiente <ChevronRight className="size-4" aria-hidden />
          </span>
        )}
      </div>
    </nav>
  );
}
