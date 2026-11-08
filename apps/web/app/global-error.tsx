"use client";

import "./globals.css";

export default function GlobalError({ error }: { error: Error }) {
  return (
    <html lang="es" data-theme="dark">
      <body className="min-h-screen px-6 py-10">
        <div role="alert" className="mx-auto max-w-[720px] rounded-md border border-line-soft bg-surface px-4 py-3 text-sm">
          <p className="font-medium">AgentSI no pudo leer la base de datos.</p>
          <p className="mono mt-2 text-xs text-muted-foreground">{error.message}</p>
        </div>
      </body>
    </html>
  );
}
