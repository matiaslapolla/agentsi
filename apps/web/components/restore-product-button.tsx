"use client";

import { useState, useTransition } from "react";
import { restoreProductAction } from "@/app/actions";

/** Restores an archived product: the same record as `agentsi restore`. */
export function RestoreProductButton({ product }: { product: string }) {
  const [pending, start] = useTransition();
  const [error, setError] = useState<string | null>(null);

  function press() {
    setError(null);
    start(async () => {
      const res = await restoreProductAction(product);
      if (!res.ok) setError(res.error);
    });
  }

  return (
    <div className="mt-2 flex items-center gap-3">
      <button
        type="button"
        onClick={press}
        disabled={pending}
        className="h-8 cursor-pointer rounded-md border border-border px-3 text-sm transition-colors hover:bg-card disabled:cursor-not-allowed disabled:opacity-50"
      >
        Restaurar producto
      </button>
      {error && <span className="text-xs text-destructive">{error}</span>}
    </div>
  );
}
