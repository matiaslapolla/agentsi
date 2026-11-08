const nf = new Intl.NumberFormat("es-AR");

export function compact(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(2).replace(".", ",")}M`;
  if (n >= 10_000) return `${(n / 1000).toFixed(1).replace(".", ",")}k`;
  return nf.format(Math.round(n));
}

export function usd(n: number): string {
  return `US$ ${n.toFixed(n < 1 ? 3 : 2).replace(".", ",")}`;
}

export function clock(sec: number): string {
  const s = Math.max(0, Math.floor(sec));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

export function dateOf(m: { id: string; created_at?: string }): string {
  if (m.created_at) {
    const d = new Date(m.created_at);
    if (!Number.isNaN(d.getTime())) {
      return d.toLocaleDateString("es-AR", { day: "2-digit", month: "short", year: "numeric" });
    }
  }
  return m.id.slice(0, 10);
}

export function plural(n: number, one: string, many: string): string {
  return n === 1 ? one : many;
}
