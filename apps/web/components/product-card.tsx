import Link from "next/link";
import { Headphones, ListChecks, Scale } from "lucide-react";
import { HealthChip } from "@/components/health-chip";
import { cn } from "@/lib/cn";
import { plural } from "@/lib/format";
import type { ProductQuest } from "@/lib/inbox";

interface Indicator {
  n: number;
  Icon: typeof ListChecks;
  label: string;
}

function Group({ name, items, muted }: { name: string; items: Indicator[]; muted?: boolean }) {
  const shown = items.filter((i) => i.n > 0);
  if (!shown.length) return null;
  return (
    <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
      <span className={cn("text-xs font-medium uppercase tracking-wide", muted ? "text-muted-foreground" : "text-foreground")}>{name}</span>
      {shown.map(({ n, Icon, label }) => (
        <span key={label} className={cn("inline-flex items-center gap-1 text-sm", muted && "text-muted-foreground")} title={`${n} ${label}`}>
          <Icon className="size-3.5 text-muted-foreground" strokeWidth={1.75} aria-hidden />
          <span className="tabular-nums">{n}</span>
          <span className="sr-only">{label}</span>
        </span>
      ))}
    </span>
  );
}

function Indicators({ p }: { p: ProductQuest }) {
  return (
    <span className="flex flex-col gap-1">
      <Group
        name="Pendientes"
        items={[
          { n: p.unmarked, Icon: ListChecks, label: "priorities sin marcar" },
          { n: p.pendingDecisions, Icon: Scale, label: "decisiones pendientes" },
          { n: p.unheard, Icon: Headphones, label: "llamadas sin escuchar" },
        ]}
      />
      <Group
        name="Hechos"
        muted
        items={[
          { n: p.marked, Icon: ListChecks, label: "priorities marcadas" },
          { n: p.resolvedDecisions, Icon: Scale, label: "decisiones resueltas" },
          { n: p.heard, Icon: Headphones, label: "llamadas escuchadas" },
        ]}
      />
    </span>
  );
}

export function ProductCard({ product, quest, note }: { product: { id: string; name: string; status: string }; quest?: ProductQuest; note: string | null }) {
  return (
    <Link
      href={`/p/${product.id}`}
      className={cn(
        "group flex h-full flex-col gap-3 rounded-paper bg-muted/60 p-5 transition-colors hover:bg-muted",
        product.status === "planned" && "text-muted-foreground",
      )}
    >
      <span className="flex items-start justify-between gap-3">
        <span className="min-w-0">
          <span className="block truncate font-semibold underline-offset-4 group-hover:underline">{product.name}</span>
          {note && <span className="block truncate text-[13px] text-muted-foreground">{note}</span>}
        </span>
        {quest && <HealthChip health={quest.health} days={quest.daysSinceMeeting} />}
      </span>
      <span className="mt-auto flex items-center justify-between gap-3">
        {quest ? <Indicators p={quest} /> : <span />}
        <span className="text-sm tabular-nums text-muted-foreground">
          {quest?.meetings ?? 0} {plural(quest?.meetings ?? 0, "reunión", "reuniones")}
        </span>
      </span>
    </Link>
  );
}
