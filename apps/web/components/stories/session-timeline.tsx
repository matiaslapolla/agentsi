import { Check, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { sessionSteps, shortDate, type StorySession } from "@/lib/stories";

const short = (sha: string | null) => (sha ? sha.slice(0, 7) : "—");

export function SessionTimeline({ session }: { session: StorySession }) {
  const steps = sessionSteps(session);
  const facts = [
    { label: "Rama", value: session.branch },
    { label: "Base", value: short(session.base) },
    { label: "Commit de tests", value: short(session.tests_commit) },
    { label: "Commit final", value: short(session.final_commit) },
    { label: "Intentos del owner", value: String(session.attempts) },
    { label: "Log", value: session.log },
  ];
  return (
    <div className="rounded-lg bg-card px-5 py-5 shadow-soft">
      <ol className="flex items-start" aria-label="Pasos de la sesión">
        {steps.map((s, i) => (
          <li key={s.state} className="relative flex flex-1 flex-col items-center text-center">
            {i > 0 && (
              <span
                className={cn(
                  "absolute right-1/2 top-[13px] h-0.5 w-full",
                  s.status === "todo" ? "bg-border" : s.status === "failed" ? "bg-status-neglected/50" : "bg-status-fresh/60",
                )}
                aria-hidden
              />
            )}
            <span
              className={cn(
                "relative grid size-7 place-items-center rounded-full",
                s.status === "done" && "bg-status-fresh text-card",
                s.status === "failed" && "bg-status-neglected text-card",
                s.status === "current" && "bg-primary/15 ring-2 ring-primary",
                s.status === "todo" && "bg-muted ring-1 ring-border",
              )}
            >
              {s.status === "done" && <Check className="size-4" strokeWidth={2.5} aria-hidden />}
              {s.status === "failed" && <X className="size-4" strokeWidth={2.5} aria-hidden />}
              {s.status === "current" && <span className="size-2.5 animate-pulse rounded-full bg-primary motion-reduce:animate-none" aria-hidden />}
            </span>
            <span
              className={cn(
                "mt-2 px-1 text-[12.5px] leading-tight",
                s.status === "todo" ? "text-muted-foreground" : "font-medium",
                s.status === "failed" && "text-status-neglected",
              )}
            >
              {s.label}
              <span className="sr-only">
                {s.status === "done" ? " (hecho)" : s.status === "current" ? " (en curso)" : s.status === "failed" ? " (falló)" : " (pendiente)"}
              </span>
            </span>
          </li>
        ))}
      </ol>

      {session.failure && (
        <p role="alert" className="mt-5 rounded-md bg-status-neglected/10 px-4 py-3 text-sm text-status-neglected">
          <span className="font-medium">Por qué falló: </span>
          {session.failure}
        </p>
      )}

      <dl className="mt-5 grid gap-x-8 gap-y-3 border-t border-border pt-4 sm:grid-cols-2 lg:grid-cols-3">
        <div>
          <dt className="label text-muted-foreground">Empezó</dt>
          <dd className="mt-0.5 text-sm">{shortDate(session.started_at)}</dd>
        </div>
        {facts.map((f) => (
          <div key={f.label} className="min-w-0">
            <dt className="label text-muted-foreground">{f.label}</dt>
            <dd className="mono mt-0.5 break-all text-xs">
              {f.value}
            </dd>
          </div>
        ))}
      </dl>
    </div>
  );
}
