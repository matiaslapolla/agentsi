import assert from "node:assert/strict";
import { test } from "node:test";
import { getPool, inTransaction, pendingMigrations } from "./db.ts";
import { testDb } from "./test-db.ts";

test("testDb applies every migration and serves getPool, with no Postgres server", async () => {
  const t = await testDb();
  try {
    assert.equal(getPool(), t.pool);
    assert.deepEqual(await pendingMigrations(getPool()), []);
    await getPool().query("insert into products (id, name, paths, status) values ($1, $2, $3, $4)", ["demo", "Demo", "[]", "active"]);
    const { rows } = await getPool().query<{ name: string }>("select name from products where id = $1", ["demo"]);
    assert.deepEqual(rows, [{ name: "Demo" }]);
  } finally {
    await t.close();
  }
});

test("a transaction on testDb rolls back on error", async () => {
  const t = await testDb();
  try {
    await assert.rejects(
      inTransaction(await getPool().connect(), async (db) => {
        await db.query("insert into products (id, name, paths, status) values ($1, $2, $3, $4)", ["gone", "Gone", "[]", "active"]);
        throw new Error("boom");
      }),
      /boom/,
    );
    assert.deepEqual((await getPool().query("select id from products")).rows, []);
  } finally {
    await t.close();
  }
});
