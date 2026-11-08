import { CalendarClock, Circle, CircleCheck, CircleDashed, Clock, TriangleAlert, type LucideIcon } from "lucide-react";
import { cn } from "@/lib/cn";
import { healthStyle, type Health, type HealthIcon as HealthIconName } from "@/lib/inbox";

const ICONS: Record<HealthIconName, LucideIcon> = { Clock, TriangleAlert, CircleCheck, Circle, CircleDashed, CalendarClock };

export function HealthIcon({ health, className }: { health: Health; className?: string }) {
  const { icon, textClass } = healthStyle(health);
  const Icon = ICONS[icon];
  return <Icon className={cn("size-4 shrink-0", textClass, className)} aria-hidden />;
}
