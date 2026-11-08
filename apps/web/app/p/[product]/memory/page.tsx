import { notFound } from "next/navigation";
import { ChevronRight, GitCommitHorizontal } from "lucide-react";
import { Crumbs, Empty } from "@/components/bits";
import { RoleAvatar } from "@/components/roster";
import { Badge } from "@/components/ui/badge";
import { findProduct } from "@/lib/registry";
import { roleMeta, sortRoles } from "@/lib/roles";
import { memoryFor, type MemoryItem } from "@/lib/workspace";

export const dynamic = "force-dynamic";

const newestFirst = (items: MemoryItem[]) => items.map((item, i) => ({ item, i })).sort((a, b) => b.item.created_at.localeCompare(a.item.created_at) || b.i - a.i).map((x) => x.item);

function ItemRow({ item }: { item: MemoryItem }) {
  return (
    <li className="flex gap-3 text-[15px]">
      <span className="mono mt-0.5 shrink-0 text-xs text-muted-foreground">{item.id}</span>
      <div className="min-w-0">
        <p className="max-w-[68ch] leading-snug">{item.text}</p>
        <p className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-muted-foreground">
          <span>{item.created_at.slice(0, 10)}</span>
          {item.kind === "lesson" && <Badge tone="accent">lección</Badge>}
          {item.helpful > 0 && <Badge tone="ok">útil ×{item.helpful}</Badge>}
          {item.harmful > 0 && <Badge tone="bad">dañino ×{item.harmful}</Badge>}
          {item.retired_why && <span>· {item.retired_why}</span>}
        </p>
      </div>
    </li>
  );
}

function MemoryItems({ items, product }: { items: MemoryItem[] | null; product: string }) {
  if (items === null) return <Empty className="py-6">Este rol todavía no escribió memory para {product}.</Empty>;
  const active = newestFirst(items.filter((i) => !i.retired_at));
  const retired = newestFirst(items.filter((i) => i.retired_at));
  return (
    <div className="space-y-5">
      {active.length === 0 ? (
        <Empty className="py-6">No hay items activos.</Empty>
      ) : (
        <ul className="space-y-3.5">
          {active.map((i) => (
            <ItemRow key={i.id} item={i} />
          ))}
        </ul>
      )}
      {retired.length > 0 && (
        <details className="group border-t border-border pt-3">
          <summary className="flex cursor-pointer items-center gap-2 text-sm text-muted-foreground">
            <ChevronRight className="chev size-4 text-muted-foreground transition-transform" />
            Retirados ({retired.length})
          </summary>
          <ul className="mt-3 space-y-3.5 text-muted-foreground">
            {retired.map((i) => (
              <ItemRow key={i.id} item={i} />
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

export default async function MemoryPage({ params }: { params: Promise<{ product: string }> }) {
  const { product: pid } = await params;
  const product = findProduct(pid);
  if (!product) notFound();
  const entries = await memoryFor(pid, sortRoles(product.roles));

  return (
    <div>
      <Crumbs items={[{ label: "Productos", href: "/" }, { label: product.name, href: `/p/${pid}` }, { label: "Memory" }]} />
      <div className="mb-8">
        <h1 className="text-3xl font-semibold tracking-[-0.02em]">Memory</h1>
        <p className="mt-1 max-w-2xl text-muted-foreground">
          Lo que cada rol aprendió de {product.name}, versionado en git para que puedas auditarlo.
        </p>
      </div>

      <div className="space-y-6">
        {entries.map((e) => (
          <section key={e.role} id={e.role} className="scroll-mt-20 md:scroll-mt-6 rounded-lg border border-border bg-card">
            <header className="flex items-center gap-3 border-b border-border px-5 py-3.5">
              <RoleAvatar role={e.role} size={32} />
              <div>
                <h2 className="font-semibold leading-tight">{roleMeta(e.role).label}</h2>
                <p className="mono text-xs text-muted-foreground">memory/{e.role}.json</p>
              </div>
            </header>
            <div className="grid gap-0 lg:grid-cols-[minmax(0,1fr)_320px]">
              <div className="px-5 py-5">
                <MemoryItems items={e.items} product={product.name} />
              </div>
              <aside className="border-t border-border px-5 py-5 lg:border-l lg:border-t-0">
                <p className="label mb-3">Historial</p>
                {e.history.length === 0 ? (
                  <p className="text-sm text-muted-foreground">Sin commits para este archivo.</p>
                ) : (
                  <ol className="space-y-3">
                    {e.history.map((h) => (
                      <li key={h.hash} className="flex gap-2.5 text-sm">
                        <GitCommitHorizontal className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
                        <div className="min-w-0">
                          <p className="leading-snug">{h.subject}</p>
                          <p className="mono mt-0.5 text-xs text-muted-foreground">
                            {h.hash} · {h.date}
                          </p>
                        </div>
                      </li>
                    ))}
                  </ol>
                )}
              </aside>
            </div>
          </section>
        ))}
      </div>
    </div>
  );
}
