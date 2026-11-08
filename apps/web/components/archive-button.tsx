"use client";

import { useState, useTransition } from "react";
import { archiveMinutesAction, restoreMinutesAction } from "@/app/actions";

/** Archives the minutes or restores them: the same record as `agentsi archive` and `agentsi restore`. The file stays on disk. */
export function ArchiveButton({ product, minutesId, archived }: { product: string; minutesId: string; archived: boolean }) {
  const [pending, start] = useTransition();
  const [error, setError] = useState<string | null>(null);

  function press() {
    setError(null);
    start(async () => {
      const res = archived ? await restoreMinutesAction(product, minutesId) : await archiveMinutesAction(product, minutesId);
      if (!res.ok) setError(res.error);
    });
  }

  return (
    <div className="mb-4 flex items-center gap-3">
      <button
        type="button"
        onClick={press}
        disabled={pending}
        className="h-9 cursor-pointer rounded-md border border-border px-3 text-sm transition-colors hover:bg-card disabled:cursor-not-allowed disabled:opacity-50"
      >
        {archived ? "Restaurar acta" : "Archivar acta"}
      </button>
      {archived && <span className="text-xs text-muted-foreground">Archivada: no aparece en las listas ni en los conteos.</span>}
      {error && <span className="text-xs text-destructive">{error}</span>}
    </div>
  );
}
