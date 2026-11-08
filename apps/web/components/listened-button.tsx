"use client";

import { useState, useTransition } from "react";
import { finishCall, removeListen } from "@/app/actions";

/** Marks the call as listened by hand (same record as a call played to the end), or unmarks it. */
export function ListenedButton({ product, minutesId, heard }: { product: string; minutesId: string; heard: boolean }) {
  const [pending, start] = useTransition();
  const [error, setError] = useState<string | null>(null);

  function press() {
    setError(null);
    start(async () => {
      const res = heard ? await removeListen(product, minutesId) : await finishCall(product, minutesId);
      if (!res.ok) setError(res.error);
    });
  }

  return (
    <div className="mb-6 flex items-center gap-3">
      <button
        type="button"
        onClick={press}
        disabled={pending}
        aria-pressed={heard}
        className="h-9 cursor-pointer rounded-md border border-border px-3 text-sm transition-colors hover:bg-card disabled:cursor-not-allowed disabled:opacity-50"
      >
        {heard ? "Quitar de escuchadas" : "Marcar como escuchada"}
      </button>
      {error && <span className="text-xs text-destructive">{error}</span>}
    </div>
  );
}
