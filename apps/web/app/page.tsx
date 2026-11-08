import Link from "next/link";
import { Boxes, ListTodo } from "lucide-react";
import { RestoreProductButton } from "@/components/restore-product-button";
import { productArchived } from "@/lib/archive";
import { Empty, GlyphHeading as Heading } from "@/components/bits";
import { Pager } from "@/components/pager";
import { ProductCard } from "@/components/product-card";
import { QuestList } from "@/components/inbox/quest-list";
import { Badge } from "@/components/ui/badge";
import { dbReachable } from "@/lib/db";
import { toDoQuests } from "@/lib/inbox";
import { loadInbox } from "@/lib/inbox-data";
import { paginate } from "@/lib/paginate";
import { readRegistry } from "@/lib/registry";
import { ROLE_ORDER, roleMeta } from "@/lib/roles";

export const dynamic = "force-dynamic";

function missingRoles(roles: string[]): string | null {
  const missing = ROLE_ORDER.filter((r) => !roles.some((id) => id.split(".")[0] === r));
  return missing.length ? `sin ${missing.map((r) => roleMeta(r).label).join(", ")}` : null;
}

type SearchParams = Promise<Record<string, string | string[] | undefined>>;

export default async function Home({ searchParams }: { searchParams: SearchParams }) {
  const params = await searchParams;
  const showArchived = params.archivados === "1";
  const all = readRegistry();
  const archivedIds = new Set((await Promise.all(all.map(async (p) => ((await productArchived(p.id)) ? p.id : null)))).filter((id): id is string => id !== null));
  const products = showArchived ? all : all.filter((p) => !archivedIds.has(p.id));
  const [inbox, reachable] = await Promise.all([loadInbox(), dbReachable()]);
  const work = paginate(toDoQuests(inbox.products), params.tareas, 8);
  const questions = new Map(inbox.minutes.map((m) => [m.id, m.question ?? ""]));
  const byId = new Map(inbox.products.map((p) => [p.id, p]));

  return (
    <div className="space-y-10">
      <div className="flex flex-wrap items-end justify-between gap-4">
        <h1 className="text-3xl font-semibold tracking-[-0.02em]">Agencia</h1>
        {!reachable && <Badge tone="warn">Base de datos sin conexión: se muestran solo los archivos del workspace</Badge>}
      </div>

      <section id="te-toca" className="scroll-mt-6">
        <Heading Icon={ListTodo} count={work.total}>Te toca</Heading>
        {work.total === 0 ? (
          <Empty>Nada te espera: todas las reuniones están marcadas, decididas y escuchadas.</Empty>
        ) : (
          <QuestList quests={work.items} products={inbox.products} questions={questions} />
        )}
        <Pager page={work} param="tareas" path="/" params={params} anchor="te-toca" label="te toca" />
      </section>

      <section>
        <Heading Icon={Boxes} count={products.length}>Productos</Heading>
        {(archivedIds.size > 0 || showArchived) && (
          <p className="mb-3 text-sm">
            <Link href={showArchived ? "/" : "/?archivados=1"} className="text-muted-foreground underline underline-offset-4 hover:text-foreground">
              {showArchived ? "Ocultar productos archivados" : "Mostrar productos archivados"}
            </Link>
          </p>
        )}
        {products.length === 0 ? (
          <Empty>
            No hay productos en el registry. Revisá <span className="mono">agentsi.config.json</span>.
          </Empty>
        ) : (
          <ul className="grid gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {products.map((p) => (
              <li key={p.id}>
                <ProductCard product={p} quest={byId.get(p.id)} note={missingRoles(p.roles)} />
                {archivedIds.has(p.id) && (
                  <div className="px-1">
                    <span className="mt-2 block text-xs text-muted-foreground">Archivado</span>
                    <RestoreProductButton product={p.id} />
                  </div>
                )}
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
