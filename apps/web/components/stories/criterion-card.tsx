import { FlaskConical } from "lucide-react";
import type { Criterion } from "@/lib/stories";

const STEPS = [
  { key: "given", label: "Dado" },
  { key: "when", label: "Cuando" },
  { key: "then", label: "Entonces" },
] as const;

export function CriterionCard({ criterion, n }: { criterion: Criterion; n: number }) {
  return (
    <li className="overflow-hidden rounded-lg bg-card shadow-soft">
      <div className="flex gap-4 px-5 pb-4 pt-5">
        <span className="mono mt-0.5 grid size-7 shrink-0 place-items-center rounded-full bg-muted text-xs text-muted-foreground">{n}</span>
        <ol className="min-w-0 flex-1 space-y-3">
          {STEPS.map((s, i) => (
            <li key={s.key} className="relative grid items-baseline gap-x-4 gap-y-0.5 sm:grid-cols-[6.5rem_1fr]">
              {i < STEPS.length - 1 && (
                <span className="absolute left-[3px] top-5 hidden h-[calc(100%-4px)] w-px bg-border sm:block" aria-hidden />
              )}
              <span className="label flex items-center gap-2 text-muted-foreground">
                <span className="hidden size-[7px] shrink-0 rounded-full bg-primary/60 sm:inline-block" aria-hidden />
                {s.label}
              </span>
              <span className="text-pretty text-[15px] leading-relaxed">{criterion[s.key]}</span>
            </li>
          ))}
        </ol>
      </div>
      <div className="space-y-1.5 bg-muted/50 px-5 py-3">
        <p className="text-sm">
          <span className="font-medium">Qué prueba: </span>
          <span className="text-muted-foreground">{criterion.checks}</span>
        </p>
        <p className="mono flex items-start gap-1.5 text-xs text-muted-foreground">
          <FlaskConical className="mt-px size-3.5 shrink-0" aria-hidden />
          <span className="break-all">{criterion.test ?? "sin test: revisión manual"}</span>
        </p>
      </div>
    </li>
  );
}
