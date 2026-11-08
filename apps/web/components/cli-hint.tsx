import type { MarkValue } from "@/lib/types";
import { Badge } from "@/components/ui/badge";

/** The web app only reads. Each write is a command that the owner runs, or tells an agent to run, in Claude Code. */
export function CliHint({ children }: { children: string }) {
  return <code className="mono select-all rounded-md border border-border bg-muted px-2 py-1 text-xs text-muted-foreground">{children}</code>;
}

export function DecideHint({ id }: { id: number | string }) {
  return <CliHint>{`agentsi decide ${id} approve|reject`}</CliHint>;
}

const MARKS: Record<MarkValue, { label: string; tone: "ok" | "warn" | "bad" }> = {
  accepted: { label: "Aceptada", tone: "ok" },
  changed: { label: "Con cambios", tone: "warn" },
  rejected: { label: "Rechazada", tone: "bad" },
};

export function MarkBadge({ mark }: { mark: MarkValue | null }) {
  const m = mark ? MARKS[mark] : null;
  return m ? <Badge tone={m.tone}>{m.label}</Badge> : <Badge>Sin marca</Badge>;
}

export function MarkState({ minutesId, idx, mark, isNew }: { minutesId: string; idx: number; mark: MarkValue | null; isNew: boolean }) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <MarkBadge mark={mark} />
      {isNew && <Badge tone="accent">Nueva</Badge>}
      <CliHint>{`agentsi mark ${minutesId} ${idx} accepted|changed|rejected`}</CliHint>
    </div>
  );
}
