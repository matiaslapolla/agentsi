import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { PGlite } from "@electric-sql/pglite";
import type pg from "pg";
import { setPool } from "./db.ts";
import { sqlDir } from "./paths.ts";

export interface TestDb {
  /** Pool-shaped wrapper of PGlite: `query`, `connect` and `end`, which is what the stores use. */
  pool: pg.Pool;
  /** Restores the real pool and closes PGlite. */
  close(): Promise<void>;
}

/**
 * An in-process Postgres with every migration of `packages/core/sql/` applied, set as the pool of `getPool()`.
 * It needs no server and no network, so story sessions can test database code.
 */
export async function testDb(): Promise<TestDb> {
  const db = await PGlite.create();
  await db.exec("create table schema_migrations (name text primary key, applied_at timestamptz not null default now())");
  for (const file of readdirSync(sqlDir).filter((f) => /^\d+_.*\.sql$/.test(f)).sort()) {
    await db.exec(readFileSync(join(sqlDir, file), "utf8"));
    await db.query("insert into schema_migrations (name) values ($1)", [file]);
  }
  // PGlite runs one statement per call with parameters; a call without them may hold several, as `pg` allows.
  const query = async (text: string, params?: unknown[]) =>
    params?.length ? db.query(text, params) : ((await db.exec(text)).at(-1) ?? { rows: [], fields: [], affectedRows: 0 });
  const client = { query, release() {} };
  const pool = { query, connect: async () => client, end: async () => {} } as unknown as pg.Pool;
  setPool(pool);
  return {
    pool,
    async close() {
      setPool(undefined);
      await db.close();
    },
  };
}
