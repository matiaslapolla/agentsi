export const DECISIONS_SIZE_PARAM = "decisiones_size";
export const DECISIONS_SIZES = [5, 10, 20] as const;
export const DEFAULT_DECISIONS_SIZE = 10;

/** Page size from a raw `?decisiones_size=` value; anything but 5, 10 or 20 falls back to 10. */
export function parseDecisionsSize(raw: string | string[] | undefined): number {
  const one = Array.isArray(raw) ? raw[0] : raw;
  const n = one !== undefined && /^\d+$/.test(one) ? Number(one) : Number.NaN;
  return (DECISIONS_SIZES as readonly number[]).includes(n) ? n : DEFAULT_DECISIONS_SIZE;
}

/** URL for choosing `size`: keeps the other params and drops the page, so the list goes back to page 1. */
export function decisionsSizeHref(
  path: string,
  params: Record<string, string | string[] | undefined>,
  pageParam: string,
  size: number,
  anchor: string,
): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (k === pageParam || k === DECISIONS_SIZE_PARAM || v === undefined) continue;
    for (const one of Array.isArray(v) ? v : [v]) q.append(k, one);
  }
  q.set(DECISIONS_SIZE_PARAM, String(size));
  return `${path}?${q.toString()}#${anchor}`;
}
