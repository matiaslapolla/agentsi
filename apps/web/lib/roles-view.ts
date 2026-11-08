export type DiffLineKind = "add" | "del" | "hunk" | "meta" | "ctx";

export function diffLines(diff: string): { kind: DiffLineKind; text: string }[] {
  return diff
    .replace(/\n$/, "")
    .split("\n")
    .map((text) => {
      if (text.startsWith("+++") || text.startsWith("---") || text.startsWith("diff ") || text.startsWith("index ")) {
        return { kind: "meta", text };
      }
      if (text.startsWith("@@")) return { kind: "hunk", text };
      if (text.startsWith("+")) return { kind: "add", text };
      if (text.startsWith("-")) return { kind: "del", text };
      return { kind: "ctx", text };
    });
}

export function isPast(due: string, today: Date = new Date()): boolean {
  return /^\d{4}-\d{2}-\d{2}$/.test(due) && due < today.toISOString().slice(0, 10);
}

export const percent = (n: number | null): string => (n === null ? "—" : `${Math.round(n * 100)}%`);
