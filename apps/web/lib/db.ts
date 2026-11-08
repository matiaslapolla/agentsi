import { getPool, isMissingSchemaError, pendingMigrations, type Queryable } from "@agentsi/core";

const quiet = new WeakSet<object>();

/**
 * The one pool of the app: core's `getPool()`, so the web app and the CLI read and write the same database.
 * A page test may put a fake pool in `globalThis.__agentsiPool`; it wins.
 */
function pool() {
  const p = (globalThis as { __agentsiPool?: ReturnType<typeof getPool> }).__agentsiPool ?? getPool();
  if (!quiet.has(p) && typeof p.on === "function") {
    p.on("error", () => {});
    quiet.add(p);
  }
  return p;
}

const CONNECTION_CODES = ["ECONNREFUSED", "ENOTFOUND", "ETIMEDOUT", "ECONNRESET", "EHOSTUNREACH", "ENETUNREACH", "EAI_AGAIN", "EPIPE"];
/** SQLSTATE classes 08 (connection exception) and 57 (operator intervention, such as a restart). */
const isConnectionError = (err: unknown): boolean => {
  const { code = "", message = "" } = (err ?? {}) as { code?: string; message?: string };
  return (
    CONNECTION_CODES.includes(code) ||
    /^(08|57)/.test(code) ||
    /timeout exceeded when trying to connect|connection terminated/i.test(message)
  );
};

/** A missing table or column means a migration is pending, and an unreachable database is the offline mode of the home page. Both give an empty list. Any other error reaches the page. */
export async function rowsFrom<T>(db: Queryable, text: string, params: unknown[] = []): Promise<T[]> {
  try {
    return (await db.query(text, params)).rows as T[];
  } catch (err) {
    if (isMissingSchemaError(err) || isConnectionError(err)) return [];
    throw err;
  }
}

export const rows = <T = Record<string, unknown>>(text: string, params: unknown[] = []): Promise<T[]> =>
  rowsFrom<T>(pool(), text, params);

export const pendingFiles = (): Promise<string[]> => pendingMigrations(pool());

export async function dbReachable(): Promise<boolean> {
  try {
    await pool().query("select 1");
    return true;
  } catch {
    return false;
  }
}
