import assert from "node:assert/strict";
import { test } from "node:test";
import { rowsFrom } from "./db";

const failing = (code?: string) => ({
  query: async () => {
    throw Object.assign(new Error("database error"), { code });
  },
});

test("rows returns an empty list for a missing table or column", async () => {
  assert.deepEqual(await rowsFrom(failing("42P01"), "select 1"), []);
  assert.deepEqual(await rowsFrom(failing("42703"), "select 1"), []);
});

test("rows returns an empty list when the database is unreachable", async () => {
  assert.deepEqual(await rowsFrom(failing("ECONNREFUSED"), "select 1"), []);
  assert.deepEqual(await rowsFrom({ query: async () => { throw new Error("timeout exceeded when trying to connect"); } }, "select 1"), []);
  assert.deepEqual(await rowsFrom({ query: async () => { throw new Error("Connection terminated unexpectedly"); } }, "select 1"), []);
  assert.deepEqual(await rowsFrom(failing("57P01"), "select 1"), []);
  assert.deepEqual(await rowsFrom(failing("EHOSTUNREACH"), "select 1"), []);
});

test("rows lets any other database error reach the page", async () => {
  await assert.rejects(rowsFrom(failing("23505"), "select 1"), /database error/);
  await assert.rejects(rowsFrom(failing(), "select 1"), /database error/);
});

test("rows returns the rows of a good query", async () => {
  assert.deepEqual(await rowsFrom({ query: async () => ({ rows: [{ id: 1 }] }) as never }, "select 1"), [{ id: 1 }]);
});
