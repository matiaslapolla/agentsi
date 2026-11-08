export type RoleId = "pm" | "social" | "dev" | "qa" | "scout";

export interface RoleMeta {
  id: RoleId;
  label: string;
  initials: string;
  blurb: string;
}

export const ROLE_ORDER: RoleId[] = ["pm", "dev", "social", "qa", "scout"];

export const ROLES: Record<RoleId, RoleMeta> = {
  pm: { id: "pm", label: "PM", initials: "PM", blurb: "Prioriza y escribe las minutes" },
  dev: { id: "dev", label: "Dev", initials: "DV", blurb: "Código, deuda y riesgo técnico" },
  social: { id: "social", label: "Social", initials: "SO", blurb: "Audiencia y distribución" },
  qa: { id: "qa", label: "QA", initials: "QA", blurb: "Calidad y regresiones" },
  scout: { id: "scout", label: "Scout", initials: "SC", blurb: "Mercado y señales externas" },
};

const roleOf = (id: string) => id.split(".")[0]!;

/** Accepts an agent id: a specialist (`dev.backend`) shows its role's meta with the focus in the label. */
export function roleMeta(id: string): RoleMeta {
  const role = roleOf(id);
  const focus = id.slice(role.length + 1);
  const base = ROLES[role as RoleId];
  if (!base) {
    return { id: id as RoleId, label: id.toUpperCase(), initials: id.slice(0, 2).toUpperCase(), blurb: "" };
  }
  return focus ? { ...base, id: id as RoleId, label: `${base.label} · ${focus}` } : base;
}

export function roleColorVar(id: string): string {
  const role = roleOf(id);
  return role in ROLES ? `var(--role-${role})` : "var(--muted-foreground)";
}

export function sortRoles(ids: string[]): string[] {
  return [...ids].sort((a, b) => {
    const ia = ROLE_ORDER.indexOf(roleOf(a) as RoleId);
    const ib = ROLE_ORDER.indexOf(roleOf(b) as RoleId);
    return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib);
  });
}
