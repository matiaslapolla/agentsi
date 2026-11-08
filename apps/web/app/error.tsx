"use client";

export default function PageError({ error }: { error: Error }) {
  return (
    <div role="alert" className="rounded-md border border-line-soft bg-surface px-4 py-3 text-sm">
      <p className="font-medium">La página no pudo leer la base de datos.</p>
      <p className="mono mt-2 text-xs text-muted-foreground">{error.message}</p>
    </div>
  );
}
