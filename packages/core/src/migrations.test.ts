import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { isMissingSchemaError, migrateHint, pendingMigrations, type Queryable } from "./db.ts";
import { sqlDir } from "./paths.ts";

const applied = (names: string[]): Queryable => ({
  query: async () => ({ rows: names.map((name) => ({ name })) }) as never,
});
const missing = (code: string): Queryable => ({
  query: async () => {
    throw Object.assign(new Error("boom"), { code });
  },
});

test("pendingMigrations lists the files that schema_migrations does not hold", async () => {
  assert.deepEqual(await pendingMigrations(applied([])), (await pendingMigrations(missing("42P01"))));
  const all = await pendingMigrations(applied([]));
  assert.ok(all.includes("005_decision_action.sql"));
  const upToFour = all.filter((f) => f < "005");
  assert.deepEqual(await pendingMigrations(applied(upToFour)), all.filter((f) => f >= "005"));
  assert.deepEqual(await pendingMigrations(applied(all)), []);
});

test("a database error that is not a missing table reaches the caller", async () => {
  await assert.rejects(pendingMigrations(missing("ECONNREFUSED")), /boom/);
  assert.equal(isMissingSchemaError(Object.assign(new Error(), { code: "42703" })), true);
  assert.equal(isMissingSchemaError(new Error("x")), false);
});

test("migration 005 adds the action column that the pending decisions read", () => {
  assert.match(readFileSync(join(sqlDir, "005_decision_action.sql"), "utf8"), /alter table decisions add column action text/);
});

test("the hint names the command and the pending files", () => {
  assert.equal(migrateHint(["005_decision_action.sql"]), "Run agentsi migrate\nPending: 005_decision_action.sql");
});
