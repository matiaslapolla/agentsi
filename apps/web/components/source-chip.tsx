import Link from "next/link";
import { FileText, GitCommitHorizontal, CircleDot, Link2, ListChecks } from "lucide-react";
import type { Source } from "@/lib/types";

const CHIP = "mono inline-flex max-w-full items-center gap-1.5 rounded-sm border border-border bg-muted px-1.5 py-0.5 text-[12px] text-muted-foreground";

export function SourceChip({ source, product }: { source: Source; product: string }) {
  const Icon =
    source.type === "commit"
      ? GitCommitHorizontal
      : source.type === "issue"
        ? CircleDot
        : source.type === "url"
          ? Link2
          : source.type === "story"
            ? ListChecks
            : FileText;
  const text =
    source.type === "commit" ? source.ref.slice(0, 8) : source.type === "issue" ? `#${source.ref.replace(/^#/, "")}` : source.ref;
  const body = (
    <>
      <Icon className="size-3.5 shrink-0 text-muted-foreground" />
      <span className="truncate">{text}</span>
    </>
  );
  if (source.type === "story") {
    return (
      <Link href={`/p/${product}/stories/${source.ref}`} title={`story: ${source.ref}`} className={`${CHIP} hover:text-foreground`}>
        {body}
      </Link>
    );
  }
  return (
    <span title={`${source.type}: ${source.ref}`} className={CHIP}>
      {body}
    </span>
  );
}
