import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import pg from "pg";
import { sqlDir } from "./paths.ts";
import { loadRegistry } from "./registry.ts";

export const DEFAULT_DATABASE_URL = "postgres://agentsi:agentsi@127.0.0.1:5561/agentsi";

let pool: pg.Pool | undefined;

/** Points `getPool()` at another pool, such as the PGlite one of `testDb()`. `undefined` goes back to the real one. */
export function setPool(next: pg.Pool | undefined): void {
  pool = next;
}

export function getPool(): pg.Pool {
  pool ??= new pg.Pool({
    connectionString: process.env.AGENTSI_DATABASE_URL ?? DEFAULT_DATABASE_URL,
    max: 5,
  });
  return pool;
}

export async function closePool(): Promise<void> {
  const p = pool;
  pool = undefined;
  await p?.end();
}

/** The one method of a pool that the checks need, so tests can pass a fake. */
export interface Queryable {
  query<R extends pg.QueryResultRow = pg.QueryResultRow>(text: string, params?: unknown[]): Promise<{ rows: R[] }>;
}

export interface TxClient extends Queryable {
  release(): void;
}

/** Runs `fn` between begin and commit. A failed rollback never hides the error that caused it. */
export async function inTransaction<T>(client: TxClient, fn: (db: Queryable) => Promise<T>): Promise<T> {
  try {
    await client.query("begin");
    const result = await fn(client);
    await client.query("commit");
    return result;
  } catch (err) {
    await client.query("rollback").catch(() => undefined);
    throw err;
  } finally {
    client.release();
  }
}

const migrationFiles = (dir: string): string[] => readdirSync(dir).filter((f) => /^\d+_.*\.sql$/.test(f)).sort();

/** Postgres codes for a missing table (42P01) or column (42703): what a pending migration causes. */
export const isMissingSchemaError = (err: unknown): boolean =>
  ["42P01", "42703"].includes((err as { code?: string } | null)?.code ?? "");

/** The sql files that `migrate` has not applied yet. A database with no `schema_migrations` table has them all pending. */
export async function pendingMigrations(db: Queryable = getPool(), dir: string = sqlDir): Promise<string[]> {
  const files = migrationFiles(dir);
  try {
    const done = new Set((await db.query<{ name: string }>("select name from schema_migrations")).rows.map((r) => r.name));
    return files.filter((f) => !done.has(f));
  } catch (err) {
    if (isMissingSchemaError(err)) return files;
    throw err;
  }
}

export const migrateHint = (pending: string[]): string => `Run agentsi migrate\nPending: ${pending.join(", ")}`;

export async function migrate(): Promise<string[]> {
  const client = await getPool().connect();
  const applied: string[] = [];
  try {
    await client.query("select pg_advisory_lock(727354)");
    await client.query(
      "create table if not exists schema_migrations (name text primary key, applied_at timestamptz not null default now())",
    );
    const done = new Set(
      (await client.query<{ name: string }>("select name from schema_migrations")).rows.map((r) => r.name),
    );
    for (const file of migrationFiles(sqlDir)) {
      if (done.has(file)) continue;
      await client.query("begin");
      try {
        await client.query(readFileSync(join(sqlDir, file), "utf8"));
        await client.query("insert into schema_migrations (name) values ($1)", [file]);
        await client.query("commit");
      } catch (err) {
        await client.query("rollback");
        throw new Error(`Migration ${file} failed: ${(err as Error).message}`);
      }
      applied.push(file);
    }
    for (const p of loadRegistry().products) {
      await client.query(
        `insert into products (id, name, paths, status) values ($1, $2, $3, $4)
         on conflict (id) do update set name = excluded.name, paths = excluded.paths, status = excluded.status`,
        [p.id, p.name, JSON.stringify(p.paths), p.status],
      );
    }
    await client.query("select pg_advisory_unlock(727354)");
  } finally {
    client.release();
  }
  return applied;
}
