import { cn } from "@/lib/cn";
import { ROLE_ORDER, roleColorVar, roleMeta } from "@/lib/roles";

export function RosterMark() {
  return (
    <span className="flex items-center -space-x-1.5" aria-hidden>
      {ROLE_ORDER.map((r) => (
        <span
          key={r}
          className="size-3.5 rounded-full ring-2 ring-background"
          style={{ background: roleColorVar(r) }}
        />
      ))}
    </span>
  );
}

export function RoleDot({ role, className }: { role: string; className?: string }) {
  return (
    <span
      className={cn("inline-block size-2 shrink-0 rounded-full", className)}
      style={{ background: roleColorVar(role) }}
      aria-hidden
    />
  );
}

export function RoleTag({ role, className }: { role: string; className?: string }) {
  const meta = roleMeta(role);
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-[13px] font-medium", className)}>
      <RoleDot role={role} />
      {meta.label}
    </span>
  );
}

export function RoleAvatar({ role, size, className }: { role: string; size?: number; className?: string }) {
  const meta = roleMeta(role);
  return (
    <span
      className={cn("grid shrink-0 place-items-center rounded-full font-semibold tracking-tight", className)}
      style={{
        ...(size ? { "--a": `${size}px` } : {}),
        width: "var(--a)",
        height: "var(--a)",
        fontSize: "calc(var(--a) * 0.36)",
        color: roleColorVar(role),
        background: `color-mix(in oklch, ${roleColorVar(role)} 18%, var(--card))`,
      } as React.CSSProperties}
      aria-hidden
    >
      {meta.initials}
    </span>
  );
}
