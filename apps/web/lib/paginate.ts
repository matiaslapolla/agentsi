export interface Page<T> {
  items: T[];
  page: number;
  pages: number;
  total: number;
  from: number;
  to: number;
}

/** One page of a list from a raw `?param=` value; out-of-range or malformed pages clamp to the nearest valid one. */
export function paginate<T>(list: T[], raw: string | string[] | undefined, size: number): Page<T> {
  const pages = Math.max(1, Math.ceil(list.length / size));
  const asked = Number.parseInt(Array.isArray(raw) ? raw[0] : (raw ?? "1"), 10);
  const page = Math.min(pages, Math.max(1, Number.isNaN(asked) ? 1 : asked));
  const start = (page - 1) * size;
  const items = list.slice(start, start + size);
  return { items, page, pages, total: list.length, from: items.length ? start + 1 : 0, to: start + items.length };
}
