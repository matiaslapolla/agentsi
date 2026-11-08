"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { ArrowRight, CircleCheck } from "lucide-react";
import { cn } from "@/lib/cn";
import type { meetingProgress } from "@/lib/inbox";

export interface MeetingProgressProps {
  progress: ReturnType<typeof meetingProgress>;
  /** One flag for each segment: true when the item is marked or decided. */
  marks: boolean[];
  decisions: boolean[];
  next: { label: string; href: string } | null;
}

function Segments({ done, prefix, label }: { done: boolean[]; prefix: string; label: string }) {
  return (
    <span className="flex items-center gap-1">
      {done.map((d, i) => (
        <a
          key={i}
          href={`#${prefix}-${i}`}
          aria-label={`${label} ${i + 1}: ${d ? "hecha" : "pendiente"}`}
          className={cn("seg block h-1.5 w-5 rounded-full sm:w-6", d ? "bg-status-fresh" : "bg-border hover:bg-muted-foreground")}
        />
      ))}
    </span>
  );
}

export function MeetingProgress({ progress, marks, decisions, next }: MeetingProgressProps) {
  const { marked, decided, cleared } = progress;
  const was = useRef(cleared);
  const [justCleared, setJustCleared] = useState(false);

  useEffect(() => {
    if (cleared && !was.current) {
      setJustCleared(true);
    }
    was.current = cleared;
  }, [cleared]);

  if (cleared) {
    return (
      <div
        className={cn(
          "mb-8 flex flex-wrap items-center justify-between gap-x-6 gap-y-3 rounded-lg border border-status-fresh/35 px-5 py-4",
          "bg-card bg-linear-to-b from-status-fresh/15 to-status-fresh/15",
          justCleared && "cleared-in sticky top-16 z-10 md:top-2",
        )}
      >
        <p className="flex items-center gap-3">
          <CircleCheck className="size-5 shrink-0 text-status-fresh" strokeWidth={2} />
          <span>
            <span className="font-semibold">Reunión cerrada.</span>{" "}
            <span className="text-muted-foreground">Todo marcado y decidido.</span>
          </span>
        </p>
        {next && (
          <Link href={next.href} className="group flex items-center gap-2 text-sm font-medium underline-offset-4 hover:underline">
            Siguiente: {next.label}
            <ArrowRight className="size-4 transition-transform group-hover:translate-x-0.5" />
          </Link>
        )}
      </div>
    );
  }

  return (
    <div className="sticky top-14 z-10 -mx-4 md:top-0 mb-8 border-b border-border bg-background px-4 py-3 sm:mx-0 sm:px-0">
      <div className="flex flex-wrap items-center gap-x-6 gap-y-2.5">
        <span className="text-sm font-medium">Esta reunión</span>
        {marks.length > 0 && (
          <span className="flex items-center gap-2.5">
            <Segments done={marks} prefix="p" label="Priority" />
            <span className="text-sm text-muted-foreground">
              <span className="tabular-nums text-foreground">
                {marked}/{marks.length}
              </span>{" "}
              marcadas
            </span>
          </span>
        )}
        {decisions.length > 0 && (
          <span className="flex items-center gap-2.5">
            <Segments done={decisions} prefix="d" label="Decisión" />
            <span className="text-sm text-muted-foreground">
              <span className="tabular-nums text-foreground">
                {decided}/{decisions.length}
              </span>{" "}
              decididas
            </span>
          </span>
        )}
      </div>
    </div>
  );
}
