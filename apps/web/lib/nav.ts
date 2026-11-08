export type NavItem = "agency" | "guide" | "how" | "product" | "minutes" | "roles" | "memory" | "stories";

export interface NavState {
  item: NavItem | null;
  /** Product group that holds the active item. */
  product: string | null;
}

/** Minutes and call pages highlight the Minutes item of their product. */
export function navState(pathname: string, products: { id: string }[]): NavState {
  if (pathname === "/") return { item: "agency", product: null };
  if (pathname === "/guia") return { item: "guide", product: null };
  if (pathname === "/como-funciona") return { item: "how", product: null };
  const [, root, id, section] = pathname.split("/");
  if (root !== "p" || !id || !products.some((p) => p.id === id)) return { item: null, product: null };
  const item: NavItem =
    section === "roles" ? "roles" : section === "memory" ? "memory" : section === "stories" ? "stories" : section === "m" ? "minutes" : "product";
  return { item, product: id };
}
