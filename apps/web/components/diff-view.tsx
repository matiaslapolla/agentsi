import { cn } from "@/lib/cn";
import { diffLines, type DiffLineKind } from "@/lib/roles-view";

const TONE: Record<DiffLineKind, string> = {
  add: "bg-status-fresh/10 text-status-fresh",
  del: "bg-status-neglected/10 text-status-neglected",
  hunk: "text-muted-foreground",
  meta: "text-muted-foreground",
  ctx: "text-muted-foreground",
};

export function DiffView({ diff, label }: { diff: string; label: string }) {
  return (
    <div
      role="region"
      aria-label={label}
      tabIndex={0}
      className="mono max-h-80 overflow-auto rounded-md border border-border bg-muted py-1.5 text-xs leading-5"
    >
      <div className="min-w-max">
        {diffLines(diff).map((l, i) => (
          <div key={i} className={cn("whitespace-pre px-3", TONE[l.kind])}>
            {l.text || " "}
          </div>
        ))}
      </div>
    </div>
  );
}
