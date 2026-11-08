import Link from "next/link";
import { ChevronRight, Headphones, ListChecks, Users } from "lucide-react";
import { plural } from "@/lib/format";
import { questHref, type ProductQuest, type Quest } from "@/lib/inbox";

const ICON = { close: ListChecks, listen: Headphones, meet: Users } as const;

function questCopy(q: Quest, p: ProductQuest | undefined) {
  if (q.kind === "close") {
    const parts = [
      q.marks && `${q.marks} ${plural(q.marks, "priority", "priorities")}`,
      q.decisions && `${q.decisions} ${plural(q.decisions, "decisión", "decisiones")}`,
    ].filter(Boolean);
    return `Cerrá la reunión: ${parts.join(" y ")}`;
  }
  if (q.kind === "listen") return "Escuchá la llamada";
  return p?.daysSinceMeeting == null ? "Primera reunión" : `Reunión nueva: ${p.daysSinceMeeting} días sin reunión`;
}

export function QuestList({
  quests,
  products,
  questions,
}: {
  quests: Quest[];
  products: ProductQuest[];
  questions: Map<string, string>;
}) {
  const byId = new Map(products.map((p) => [p.id, p]));
  return (
    <ul className="grid gap-3 md:grid-cols-2">
      {quests.map((q, i) => {
        const Icon = ICON[q.kind];
        const p = byId.get(q.product);
        const context = q.kind === "meet" ? "Abrí /agentsi en el producto y convocá a los roles" : questions.get(q.minutesId ?? "");
        return (
          <li key={`${q.kind}-${q.product}-${q.minutesId ?? i}`}>
            <Link
              href={questHref(q)}
              className="group flex h-full items-start gap-3 rounded-lg bg-card p-4 shadow-soft transition-shadow hover:shadow-md"
            >
              <Icon className="mt-0.5 size-[18px] shrink-0 text-primary" strokeWidth={1.75} aria-hidden />
              <span className="min-w-0 flex-1">
                <span className="block font-medium">{questCopy(q, p)}</span>
                <span className="block text-sm text-muted-foreground">{p?.name ?? q.product}</span>
                {context && <p className="mt-1 line-clamp-2 text-[13px] text-muted-foreground">{context}</p>}
              </span>
              <ChevronRight className="mt-0.5 size-4 shrink-0 text-muted-foreground transition-transform group-hover:translate-x-0.5" aria-hidden />
            </Link>
          </li>
        );
      })}
    </ul>
  );
}
