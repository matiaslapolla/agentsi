import { CliHint } from "@/components/cli-hint";
import { pendingFiles } from "@/lib/db";

/** A database error here is left to the page that needs the database. */
export async function MigrationPanel() {
  const pending = await pendingFiles().catch(() => []);
  if (!pending.length) return null;
  return (
    <div role="alert" className="mb-8 rounded-md border border-border bg-card px-4 py-3 text-sm">
      <p className="font-medium">Faltan migraciones de la base de datos. Algunas listas salen vacías hasta aplicarlas.</p>
      <p className="mt-2">
        <CliHint>Run agentsi migrate</CliHint>
      </p>
      <p className="mono mt-2 text-xs text-muted-foreground">{pending.join(", ")}</p>
    </div>
  );
}
