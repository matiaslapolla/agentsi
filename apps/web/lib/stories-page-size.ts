export const STORIES_PAGE_PARAM = "stories";
export const STORIES_SIZE_PARAM = "stories_size";
export const STORIES_SIZES = [5, 10, 20] as const;
export const DEFAULT_STORIES_SIZE = 10;

/** Page size from a raw `?stories_size=` value; anything but 5, 10 or 20 falls back to 10. */
export function parseStoriesSize(raw: string | string[] | undefined): number {
  const one = Array.isArray(raw) ? raw[0] : raw;
  const n = one !== undefined && /^\d+$/.test(one) ? Number(one) : Number.NaN;
  return (STORIES_SIZES as readonly number[]).includes(n) ? n : DEFAULT_STORIES_SIZE;
}

/** URL for choosing `size`: keeps the other params and drops the page, so the list goes back to page 1. */
export function storiesSizeHref(path: string, params: Record<string, string | string[] | undefined>, size: number): string {
  const q = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (k === STORIES_PAGE_PARAM || k === STORIES_SIZE_PARAM || v === undefined) continue;
    for (const one of Array.isArray(v) ? v : [v]) q.append(k, one);
  }
  q.set(STORIES_SIZE_PARAM, String(size));
  return `${path}?${q.toString()}`;
}
