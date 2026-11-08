import Link from "next/link";
import { ChevronRight, type LucideIcon } from "lucide-react";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/cn";
import type { MarkSummary } from "@/lib/types";

export function Crumbs({ items }: { items: { label: string; href?: string }[] }) {
  return (
    <nav aria-label="Ruta" className="mb-5 flex flex-wrap items-center gap-1 text-[13px] text-muted-foreground">
      {items.map((it, i) => (
        <span key={i} className="flex items-center gap-1">
          {i > 0 && <ChevronRight className="size-3.5 text-muted-foreground" />}
          {it.href ? (
            <Link href={it.href} className="hover:text-foreground">
              {it.label}
            </Link>
          ) : (
            <span className="text-foreground">{it.label}</span>
          )}
        </span>
      ))}
    </nav>
  );
}

export function SectionTitle({ children, aside }: { children: React.ReactNode; aside?: React.ReactNode }) {
  return (
    <div className="mb-2 flex items-baseline justify-between gap-4">
      <h2 className="text-[17px] font-semibold tracking-[-0.01em]">{children}</h2>
      {aside != null && <span className="text-[13px] tabular-nums text-muted-foreground">{aside}</span>}
    </div>
  );
}

/** Section heading with the 48px glyph of DESIGN.md. */
export function GlyphHeading({ Icon, count, children }: { Icon: LucideIcon; count?: number; children: React.ReactNode }) {
  return (
    <div className="mb-4 flex items-center gap-3">
      <span className="grid size-12 shrink-0 place-items-center rounded-lg bg-primary/10 text-primary">
        <Icon className="size-6" strokeWidth={1.75} aria-hidden />
      </span>
      <h2 className="text-[17px] font-semibold tracking-[-0.01em]">{children}</h2>
      {count !== undefined && <span className="text-[13px] tabular-nums text-muted-foreground">{count}</span>}
    </div>
  );
}

export function Empty({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <p className={cn("border-y border-border py-5 text-sm text-muted-foreground", className)}>{children}</p>
  );
}

export function StatusBadge({ status }: { status?: string }) {
  if (status === "valid") return <Badge tone="ok">válida</Badge>;
  if (status === "invalid") return <Badge tone="bad">inválida</Badge>;
  return <Badge>sin estado</Badge>;
}

export function MarksSummaryInline({ s, total }: { s: MarkSummary; total: number }) {
  if (!total) return <span className="text-muted-foreground">sin priorities</span>;
  const parts = [
    s.accepted && { n: s.accepted, label: "aceptadas", cls: "text-status-fresh" },
    s.changed && { n: s.changed, label: "con cambios", cls: "text-status-waiting" },
    s.rejected && { n: s.rejected, label: "rechazadas", cls: "text-status-neglected" },
    s.unmarked && { n: s.unmarked, label: "sin marcar", cls: "text-muted-foreground" },
  ].filter(Boolean) as { n: number; label: string; cls: string }[];
  return (
    <span className="flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[13px]">
      {parts.map((p) => (
        <span key={p.label} className={p.cls}>
          <span className="mono">{p.n}</span> {p.label}
        </span>
      ))}
      {s.fresh > 0 && (
        <Badge tone="accent">
          <span className="mono">{s.fresh}</span> nueva{s.fresh > 1 ? "s" : ""}
        </Badge>
      )}
    </span>
  );
}

export function Markdown({ children }: { children: string }) {
  return (
    <div className="md text-[14.5px] leading-relaxed">
      <ReactMarkdown remarkPlugins={[remarkGfm]}>{children}</ReactMarkdown>
    </div>
  );
}
