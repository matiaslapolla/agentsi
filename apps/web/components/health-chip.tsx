import { HealthIcon } from "@/components/health-icon";
import { healthStyle, type Health } from "@/lib/inbox";

export function HealthChip({ health, days }: { health: Health; days: number | null }) {
  return (
    <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-muted px-2.5 py-1 text-xs font-medium text-foreground">
      <HealthIcon health={health} className="size-3.5" />
      {healthStyle(health, days).label}
    </span>
  );
}
